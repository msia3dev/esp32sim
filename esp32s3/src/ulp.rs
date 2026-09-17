//! ESP32-S3 ULP-FSM RTC-memory adapter and instruction scheduler.

use esp_periph::{RtcCntl, UlpArchitecture, UlpController, UlpState};
use ulp_fsm::{decode, step, Bus, Cpu, Event};

const RC_FAST_HZ: u64 = 17_500_000;
const XTAL_D2_HZ: u64 = 20_000_000;
const CPU_HZ: u64 = crate::periph::CPU_HZ;
const VPAGE_SHIFT: usize = xtensa_lx7::bus::VPAGE_SHIFT as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    OutOfRange(u16),
}

pub(crate) struct RtcSlowBus<'a> {
    memory: &'a mut [u8],
    versions: &'a mut [u32],
    version_base: usize,
}

impl<'a> RtcSlowBus<'a> {
    pub(crate) fn new(memory: &'a mut [u8], versions: &'a mut [u32], version_base: u32) -> Self {
        Self {
            memory,
            versions,
            version_base: version_base as usize,
        }
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

    pub fn reconcile(
        &mut self,
        controller: &mut UlpController,
        bus: &mut impl Bus<Error = MemoryError>,
    ) {
        if controller.state == UlpState::Reset {
            self.cpu.reset_architecture(controller.entry_pc);
        }
        if controller.architecture != UlpArchitecture::Fsm || controller.state != UlpState::Running
        {
            self.active = false;
            self.remaining_ulp_cycles = 0;
            self.phase = 0;
            return;
        }
        if self.active {
            return;
        }
        self.cpu.restart(controller.entry_pc);
        self.phase = 0;
        self.active = true;
        self.prime(controller, bus);
    }

    pub fn advance(
        &mut self,
        cpu_cycles: u32,
        fast_hz: u64,
        controller: &mut UlpController,
        bus: &mut impl Bus<Error = MemoryError>,
    ) {
        let was_active = self.active;
        self.reconcile(controller, bus);
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
                Ok(Event::Continue) => self.prime(controller, bus),
                Ok(Event::Wake) => {
                    self.wake_requests += 1;
                    self.prime(controller, bus);
                }
                Ok(Event::Halt) => {
                    controller.halt();
                    self.active = false;
                }
                Err(trap) => {
                    self.traps += 1;
                    self.last_trap = Some(format!("{trap:?}"));
                    self.cpu.halted = true;
                    controller.halt();
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

    fn prime(&mut self, controller: &mut UlpController, bus: &mut impl Bus<Error = MemoryError>) {
        match bus.read_word(self.cpu.pc) {
            Ok(raw) => match decode(raw).cycles() {
                Some(cycles) => self.remaining_ulp_cycles = cycles,
                None => self.trap_before_execute(
                    controller,
                    format!("unsupported instruction {raw:#010x} at pc {}", self.cpu.pc),
                ),
            },
            Err(error) => self.trap_before_execute(controller, format!("{error:?}")),
        }
    }

    fn trap_before_execute(&mut self, controller: &mut UlpController, message: String) {
        self.traps += 1;
        self.last_trap = Some(message);
        self.cpu.halted = true;
        controller.halt();
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
        let mut memory = vec![0u8; 8192];
        let mut versions = vec![0u32; 64];
        let mut bus = RtcSlowBus::new(&mut memory, &mut versions, 4);
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
        let mut controller = UlpController::new();
        controller.state = UlpState::Reset;
        controller.entry_pc = 7;
        let mut memory = vec![0u8; 8192];
        let mut versions = vec![0u32; 64];
        let mut bus = RtcSlowBus::new(&mut memory, &mut versions, 4);

        engine.reconcile(&mut controller, &mut bus);

        assert_eq!(engine.cpu.regs, [0; 4]);
        assert_eq!((engine.cpu.pc, engine.cpu.stage), (7, 0));
        assert!(!engine.cpu.overflow);
        assert_eq!((engine.cpu.insn_count, engine.cpu.cycle_count), (12, 34));
    }
}
