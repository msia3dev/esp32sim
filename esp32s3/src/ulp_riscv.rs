//! ESP32-S3 ULP RISC-V wrapper over the shared RV32IMC interpreter.

use emu_core::Fault;
use esp_periph::{RtcCntl, UlpArchitecture, UlpState};
use riscv_rv32::{step, Bus, Cpu};

const CPU_HZ: u64 = crate::periph::CPU_HZ;
const RTC_MEM_SIZE: u32 = 0x2000;
const VPAGE_SHIFT: usize = xtensa_lx7::bus::VPAGE_SHIFT as usize;

pub(crate) struct UlpRiscVBus<'a> {
    rtc: &'a mut RtcCntl,
    memory: &'a mut [u8],
    versions: &'a mut [u32],
    version_base: usize,
    gpio_changes: Vec<(u8, bool)>,
    wake_requested: bool,
}

impl<'a> UlpRiscVBus<'a> {
    pub(crate) fn new(rtc: &'a mut RtcCntl, memory: &'a mut [u8], versions: &'a mut [u32], version_base: u32) -> Self {
        Self { rtc, memory, versions, version_base: version_base as usize, gpio_changes: Vec::new(), wake_requested: false }
    }

    pub(crate) fn take_gpio_changes(&mut self) -> Vec<(u8, bool)> { std::mem::take(&mut self.gpio_changes) }
    pub(crate) fn take_wake_request(&mut self) -> bool { std::mem::take(&mut self.wake_requested) }

    fn memory_offset(&self, addr: u32, width: usize) -> Result<usize, Fault> {
        if !(addr as usize).is_multiple_of(width) { return Err(Fault::Misaligned); }
        let offset = addr as usize;
        if addr < RTC_MEM_SIZE && offset + width <= self.memory.len() { Ok(offset) } else { Err(Fault::Unmapped) }
    }

    fn peripheral_offset(addr: u32) -> Option<u32> {
        match addr {
            0x8000..=0x83ff => Some(addr - 0x8000),
            0xa400..=0xa7ff => Some(0x400 + addr - 0xa400),
            0xc800..=0xcbff => Some(0x800 + addr - 0xc800),
            0xec00..=0xefff => Some(0xc00 + addr - 0xec00),
            _ => None,
        }
    }

    fn read(&mut self, addr: u32, width: usize) -> Result<u32, Fault> {
        if !(addr as usize).is_multiple_of(width) { return Err(Fault::Misaligned); }
        if let Some(off) = Self::peripheral_offset(addr & !3) {
            let word = self.rtc.read(off);
            return Ok(match width { 1 => word >> ((addr & 3) * 8) & 0xff, 2 => word >> ((addr & 2) * 8) & 0xffff, _ => word });
        }
        let offset = self.memory_offset(addr, width)?;
        Ok(match width {
            1 => u32::from(self.memory[offset]),
            2 => u32::from(u16::from_le_bytes(self.memory[offset..offset + 2].try_into().unwrap())),
            _ => u32::from_le_bytes(self.memory[offset..offset + 4].try_into().unwrap()),
        })
    }

    fn write(&mut self, addr: u32, value: u32, width: usize) -> Result<(), Fault> {
        if let Some(off) = Self::peripheral_offset(addr & !3) {
            if !(addr as usize).is_multiple_of(width) { return Err(Fault::Misaligned); }
            let old_out = self.rtc.ram.read(0x400);
            let old_enable = self.rtc.ram.read(0x40c);
            let word = if width == 4 { value } else {
                let old = self.rtc.read(off);
                let shift = if width == 1 { (addr & 3) * 8 } else { (addr & 2) * 8 };
                let mask = if width == 1 { 0xff } else { 0xffff };
                (old & !(mask << shift)) | ((value & mask) << shift)
            };
            self.rtc.write(off, word);
            self.wake_requested |= off == 0x18 && word & 1 != 0;
            self.capture_gpio_changes(old_out, old_enable);
            return Ok(());
        }
        let offset = self.memory_offset(addr, width)?;
        match width {
            1 => self.memory[offset] = value as u8,
            2 => self.memory[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes()),
            _ => self.memory[offset..offset + 4].copy_from_slice(&value.to_le_bytes()),
        }
        let first = self.version_base + (offset >> VPAGE_SHIFT);
        let last = self.version_base + ((offset + width - 1) >> VPAGE_SHIFT);
        for page in first..=last { if let Some(version) = self.versions.get_mut(page) { *version = version.wrapping_add(1); } }
        Ok(())
    }

    fn capture_gpio_changes(&mut self, old_out: u32, old_enable: u32) {
        let new_out = self.rtc.ram.read(0x400);
        let new_enable = self.rtc.ram.read(0x40c);
        let changed = ((old_out ^ new_out) & new_enable) | (!old_enable & new_enable);
        for channel in 0..22u8 {
            let bit = 1u32 << (10 + channel);
            let pad = 0x484 + u32::from(channel) * 4;
            if changed & bit != 0 && self.rtc.ram.read(pad) & (1 << 19) != 0 { self.gpio_changes.push((channel, new_out & bit != 0)); }
        }
    }
}

impl Bus for UlpRiscVBus<'_> {
    fn read8(&mut self, addr: u32) -> Result<u8, Fault> { Ok(self.read(addr, 1)? as u8) }
    fn read16(&mut self, addr: u32) -> Result<u16, Fault> { Ok(self.read(addr, 2)? as u16) }
    fn read32(&mut self, addr: u32) -> Result<u32, Fault> { self.read(addr, 4) }
    fn write8(&mut self, addr: u32, value: u8) -> Result<(), Fault> { self.write(addr, u32::from(value), 1) }
    fn write16(&mut self, addr: u32, value: u16) -> Result<(), Fault> { self.write(addr, u32::from(value), 2) }
    fn write32(&mut self, addr: u32, value: u32) -> Result<(), Fault> { self.write(addr, value, 4) }
    fn fetch(&mut self, pc: u32) -> Result<[u8; 4], Fault> {
        let offset = self.memory_offset(pc, 1)?;
        let mut bytes = [0; 4];
        for (i, byte) in bytes.iter_mut().enumerate() { if let Some(value) = self.memory.get(offset + i) { *byte = *value; } }
        Ok(bytes)
    }
    fn page_versions(&self) -> &[u32] { self.versions }
    fn code_page(&mut self, pc: u32) -> u32 { self.version_base as u32 + (pc >> VPAGE_SHIFT) }
}

pub struct UlpRiscVEngine {
    pub cpu: Cpu,
    pub traps: u64,
    pub wake_requests: u64,
    pub last_trap: Option<String>,
    active: bool,
    phase: u64,
}

impl UlpRiscVEngine {
    pub fn new() -> Self { Self { cpu: Cpu::new(), traps: 0, wake_requests: 0, last_trap: None, active: false, phase: 0 } }

    pub(crate) fn reconcile(&mut self, bus: &mut UlpRiscVBus<'_>) {
        if bus.rtc.ulp.architecture != UlpArchitecture::RiscV || bus.rtc.ulp.state != UlpState::Running {
            self.active = false;
            self.phase = 0;
            return;
        }
        if self.active { return; }
        self.cpu.reset();
        self.cpu.pc = 0;
        self.active = true;
        self.phase = 0;
    }

    pub(crate) fn advance(&mut self, cpu_cycles: u32, fast_hz: u64, bus: &mut UlpRiscVBus<'_>) {
        let was_active = self.active;
        self.reconcile(bus);
        if !self.active || !was_active { return; }
        let numerator = self.phase + u64::from(cpu_cycles) * fast_hz;
        let available = numerator / CPU_HZ;
        self.phase = numerator % CPU_HZ;
        for _ in 0..available {
            match step(&mut self.cpu, bus) {
                Ok(()) => {}
                Err(trap) => {
                    self.traps += 1;
                    self.last_trap = Some(format!("{trap:?}"));
                    bus.rtc.raise_cocpu_trap_interrupt();
                    bus.rtc.ulp.halt();
                    self.active = false;
                    break;
                }
            }
            if bus.take_wake_request() { self.wake_requests += 1; }
            if bus.rtc.ulp.state != UlpState::Running { self.active = false; break; }
        }
    }

    pub fn cpu_cycles_until_deadline(&self, fast_hz: u64) -> Option<u32> {
        if !self.active { return None; }
        Some((CPU_HZ - self.phase).div_ceil(fast_hz).clamp(1, u64::from(u32::MAX)) as u32)
    }
}

impl Default for UlpRiscVEngine { fn default() -> Self { Self::new() } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_bus_maps_rtc_memory_and_converted_register_windows() {
        let mut rtc = RtcCntl::new();
        let mut memory = vec![0; RTC_MEM_SIZE as usize];
        let mut versions = vec![0; 64];
        let mut bus = UlpRiscVBus::new(&mut rtc, &mut memory, &mut versions, 4);
        bus.write32(4, 0x1234_5678).unwrap();
        assert_eq!(bus.read32(4), Ok(0x1234_5678));
        bus.write32(0x8134, 9 << 8).unwrap();
        assert_eq!(bus.rtc.ram.read(0x134), 9 << 8);
        assert_eq!(bus.read32(0x10000), Err(Fault::Unmapped));
    }
}
