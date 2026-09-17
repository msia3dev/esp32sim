//! ESP32-S3 ULP-FSM RTC-memory adapter and instruction scheduler.

use esp_periph::{RtcCntl, UlpArchitecture, UlpState};
use ulp_fsm::{decode, step, Bus, Cpu, Event, Insn, Kind};

const RC_FAST_HZ: u64 = 17_500_000;
const XTAL_D2_HZ: u64 = 20_000_000;
const CPU_HZ: u64 = crate::periph::CPU_HZ;
const VPAGE_SHIFT: usize = xtensa_lx7::bus::VPAGE_SHIFT as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    OutOfRange(u16),
}

pub(crate) struct RtcSlowBus<'a> {
    rtc: &'a mut RtcCntl,
    memory: &'a mut [u8],
    versions: &'a mut [u32],
    version_base: usize,
    gpio_changes: Vec<(u8, bool)>,
}

impl<'a> RtcSlowBus<'a> {
    pub(crate) fn new(
        rtc: &'a mut RtcCntl,
        memory: &'a mut [u8],
        versions: &'a mut [u32],
        version_base: u32,
    ) -> Self {
        Self {
            rtc,
            memory,
            versions,
            version_base: version_base as usize,
            gpio_changes: Vec::new(),
        }
    }

    pub(crate) fn take_gpio_changes(&mut self) -> Vec<(u8, bool)> {
        std::mem::take(&mut self.gpio_changes)
    }
}

impl Bus for RtcSlowBus<'_> {
    type Error = MemoryError;

    fn read_word(&mut self, address: u16) -> Result<u32, Self::Error> {
        let offset = address as usize * 4;
        let bytes = self
            .memory
            .get(offset..offset + 4)
            .ok_or(MemoryError::OutOfRange(address))?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn write_word(&mut self, address: u16, value: u32) -> Result<(), Self::Error> {
        let offset = address as usize * 4;
        let bytes = self
            .memory
            .get_mut(offset..offset + 4)
            .ok_or(MemoryError::OutOfRange(address))?;
        bytes.copy_from_slice(&value.to_le_bytes());
        let page = self.version_base + (offset >> VPAGE_SHIFT);
        if let Some(version) = self.versions.get_mut(page) {
            *version = version.wrapping_add(1);
        }
        if offset & ((1 << VPAGE_SHIFT) - 1) < 3 && page > self.version_base {
            if let Some(version) = self.versions.get_mut(page - 1) {
                *version = version.wrapping_add(1);
            }
        }
        Ok(())
    }

    fn read_reg(&mut self, peripheral: u8, address: u8) -> Option<u32> {
        let offset = u32::from(peripheral) * 0x400 + u32::from(address) * 4;
        (offset < 0x1000).then(|| self.rtc.read(offset))
    }

    fn write_reg(&mut self, peripheral: u8, address: u8, value: u32) -> bool {
        let offset = u32::from(peripheral) * 0x400 + u32::from(address) * 4;
        if offset >= 0x1000 {
            return false;
        }
        let old_out = self.rtc.ram.read(0x400);
        let old_enable = self.rtc.ram.read(0x40c);
        self.rtc.write(offset, value);
        if peripheral == 1 && address <= 5 {
            let new_out = self.rtc.ram.read(0x400);
            let new_enable = self.rtc.ram.read(0x40c);
            let changed = ((old_out ^ new_out) & new_enable) | (!old_enable & new_enable);
            for channel in 0..22u8 {
                let bit = 1u32 << (10 + channel);
                let pad = 0x484 + u32::from(channel) * 4;
                if changed & bit != 0 && self.rtc.ram.read(pad) & (1 << 19) != 0 {
                    self.gpio_changes.push((channel, new_out & bit != 0));
                }
            }
        }
        true
    }

    fn instruction_cycles(&mut self, insn: Insn) -> Option<u32> {
        match insn.kind {
            Kind::Adc { .. } => {
                let amp1 = self.rtc.ram.read(0x818);
                let amp2 = self.rtc.ram.read(0x81c);
                let waits = (amp1 & 0xffff).max(1) + (amp1 >> 16).max(1) + (amp2 >> 16).max(1);
                Some(
                    27 + waits
                        + u32::from(self.rtc.ulp_adc_sample_cycle)
                        + u32::from(self.rtc.ulp_adc_sample_bits),
                )
            }
            Kind::Tsens { delay, .. } => {
                let divider = ((self.rtc.ram.read(0x850) >> 14) & 0xff).max(1);
                Some(6 + u32::from(delay) + 3 * divider)
            }
            Kind::I2c { .. } => {
                let low = (self.rtc.ram.read(0xc00) & 0x000f_ffff).max(1);
                let high = (self.rtc.ram.read(0xc14) & 0x000f_ffff).max(1);
                let start = self.rtc.ram.read(0xc1c) & 0x000f_ffff;
                let stop = self.rtc.ram.read(0xc20) & 0x000f_ffff;
                Some(4 + start + stop + 27 * (low + high))
            }
            _ => insn.cycles(),
        }
    }

    fn adc(&mut self, sar: u8, mux: u8) -> Option<u16> {
        self.rtc
            .ulp_adc
            .get(sar as usize)?
            .get(mux.checked_sub(1)? as usize)
            .copied()
    }

    fn tsens(&mut self) -> Option<u16> {
        Some(self.rtc.ulp_tsens)
    }

    fn i2c_read(&mut self, bus: u8, address: u8) -> Option<u8> {
        self.rtc
            .ulp_i2c
            .get(bus as usize)
            .map(|registers| registers[address as usize])
    }

    fn i2c_write(&mut self, bus: u8, address: u8, value: u8) -> bool {
        let Some(registers) = self.rtc.ulp_i2c.get_mut(bus as usize) else {
            return false;
        };
        registers[address as usize] = value;
        true
    }
}

#[derive(Debug)]
pub struct UlpFsmEngine {
    pub cpu: Cpu,
    pub traps: u64,
    pub wake_requests: u64,
    pub last_trap: Option<String>,
    active: bool,
    remaining_ulp_cycles: u32,
    phase: u64,
}

impl UlpFsmEngine {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(0),
            traps: 0,
            wake_requests: 0,
            last_trap: None,
            active: false,
            remaining_ulp_cycles: 0,
            phase: 0,
        }
    }

    pub fn rtc_fast_hz(rtc: &RtcCntl) -> u64 {
        let clock = rtc.ram.read(0x74);
        if clock & (1 << 29) == 0 {
            XTAL_D2_HZ
        } else {
            RC_FAST_HZ / (((clock >> 12) & 7) as u64 + 1)
        }
    }

    pub(crate) fn reconcile(&mut self, bus: &mut RtcSlowBus<'_>) {
        if bus.rtc.ulp.state == UlpState::Reset {
            self.cpu.reset_architecture(bus.rtc.ulp.entry_pc);
        }
        if bus.rtc.ulp.architecture != UlpArchitecture::Fsm
            || bus.rtc.ulp.state != UlpState::Running
        {
            self.active = false;
            self.remaining_ulp_cycles = 0;
            self.phase = 0;
            return;
        }
        if self.active {
            return;
        }
        self.cpu.restart(bus.rtc.ulp.entry_pc);
        self.phase = 0;
        self.active = true;
        self.prime(bus);
    }

    pub(crate) fn advance(&mut self, cpu_cycles: u32, fast_hz: u64, bus: &mut RtcSlowBus<'_>) {
        let was_active = self.active;
        self.reconcile(bus);
        if !self.active || !was_active {
            return;
        }

        let numerator = self.phase + u64::from(cpu_cycles) * fast_hz;
        let mut available = (numerator / CPU_HZ) as u32;
        self.phase = numerator % CPU_HZ;
        while self.active && available >= self.remaining_ulp_cycles {
            available -= self.remaining_ulp_cycles;
            self.remaining_ulp_cycles = 0;
            match step(&mut self.cpu, bus) {
                Ok(Event::Continue) => self.prime(bus),
                Ok(Event::Wake) => {
                    self.wake_requests += 1;
                    self.prime(bus);
                }
                Ok(Event::Halt) => {
                    bus.rtc.ulp.halt();
                    self.active = false;
                }
                Err(trap) => {
                    self.traps += 1;
                    self.last_trap = Some(format!("{trap:?}"));
                    self.cpu.halted = true;
                    bus.rtc.ulp.halt();
                    self.active = false;
                }
            }
        }
        if self.active && available > 0 {
            self.remaining_ulp_cycles -= available;
        }
    }

    pub fn cpu_cycles_until_deadline(&self, fast_hz: u64) -> Option<u32> {
        if !self.active || self.remaining_ulp_cycles == 0 {
            return None;
        }
        let numerator = u64::from(self.remaining_ulp_cycles) * CPU_HZ - self.phase;
        Some(numerator.div_ceil(fast_hz).clamp(1, u64::from(u32::MAX)) as u32)
    }

    fn prime(&mut self, bus: &mut RtcSlowBus<'_>) {
        match bus.read_word(self.cpu.pc) {
            Ok(raw) => match bus.instruction_cycles(decode(raw)) {
                Some(cycles) => self.remaining_ulp_cycles = cycles,
                None => self.trap_before_execute(
                    bus,
                    format!("unsupported instruction {raw:#010x} at pc {}", self.cpu.pc),
                ),
            },
            Err(error) => self.trap_before_execute(bus, format!("{error:?}")),
        }
    }

    fn trap_before_execute(&mut self, bus: &mut RtcSlowBus<'_>, message: String) {
        self.traps += 1;
        self.last_trap = Some(message);
        self.cpu.halted = true;
        bus.rtc.ulp.halt();
        self.active = false;
        self.remaining_ulp_cycles = 0;
    }
}

impl Default for UlpFsmEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtc_slow_store_bumps_the_written_code_page_version() {
        let mut rtc = RtcCntl::new();
        let mut memory = vec![0u8; 8192];
        let mut versions = vec![0u32; 64];
        let mut bus = RtcSlowBus::new(&mut rtc, &mut memory, &mut versions, 4);
        bus.write_word(64, 0x1234_5678).unwrap();
        assert_eq!(bus.read_word(64), Ok(0x1234_5678));
        assert_eq!(versions[5], 1);
        assert_eq!(
            versions[4], 1,
            "a word at the page start invalidates a prior cross-page decode"
        );
    }

    #[test]
    fn controller_reset_clears_architecture_state_but_keeps_diagnostics() {
        let mut engine = UlpFsmEngine::new();
        engine.cpu.regs = [1, 2, 3, 4];
        engine.cpu.stage = 9;
        engine.cpu.overflow = true;
        engine.cpu.insn_count = 12;
        engine.cpu.cycle_count = 34;
        let mut rtc = RtcCntl::new();
        rtc.ulp.state = UlpState::Reset;
        rtc.ulp.entry_pc = 7;
        let mut memory = vec![0u8; 8192];
        let mut versions = vec![0u32; 64];
        let mut bus = RtcSlowBus::new(&mut rtc, &mut memory, &mut versions, 4);

        engine.reconcile(&mut bus);

        assert_eq!(engine.cpu.regs, [0; 4]);
        assert_eq!((engine.cpu.pc, engine.cpu.stage), (7, 0));
        assert!(!engine.cpu.overflow);
        assert_eq!((engine.cpu.insn_count, engine.cpu.cycle_count), (12, 34));
    }
}
