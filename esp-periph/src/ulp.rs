//! ESP32-S3 ULP controller lifecycle shared by the FSM and RISC-V engines.

const DEFAULT_WAKE_PERIOD: u32 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UlpArchitecture {
    Fsm,
    RiscV,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UlpState {
    Reset,
    Off,
    WakeDelay,
    Running,
    HaltDelay,
    Halted,
}

/// Register-facing state for the autonomous RTC-domain ULP engine.
///
/// Instruction execution is intentionally outside this type. Architecture cores consume a
/// start event and report halt/trap completion back to this controller in later milestones.
#[derive(Debug)]
pub struct UlpController {
    pub architecture: UlpArchitecture,
    pub state: UlpState,
    pub entry_pc: u16,
    pub memory_base: u16,
    pub memory_words: u16,
    pub starts: u64,
    pub halts: u64,
    debug: bool,
    timer_enabled: bool,
    fsm_clock_force: bool,
    cocpu_clock_force: bool,
    clock_gate: bool,
    wake_period: u32,
    wake_remaining: Option<u32>,
}

impl UlpController {
    pub fn new() -> Self {
        Self {
            architecture: UlpArchitecture::Fsm,
            state: UlpState::Off,
            entry_pc: 0,
            memory_base: 512,
            memory_words: 512,
            starts: 0,
            halts: 0,
            debug: false,
            timer_enabled: false,
            fsm_clock_force: false,
            cocpu_clock_force: false,
            clock_gate: false,
            wake_period: DEFAULT_WAKE_PERIOD,
            wake_remaining: None,
        }
    }

    pub fn configure_timer(&mut self, value: u32) {
        self.entry_pc = (value & 0x7ff) as u16;
        let enabled = value & (1 << 31) != 0;
        if enabled && !self.timer_enabled {
            self.arm_timer();
        } else if !enabled {
            self.wake_remaining = None;
            if self.state == UlpState::WakeDelay {
                self.state = UlpState::Off;
            }
        }
        self.timer_enabled = enabled;
    }

    pub fn configure_control(&mut self, value: u32) {
        self.memory_base = (value & 0x7ff) as u16;
        self.memory_words = ((value >> 11) & 0x7ff) as u16;
        self.fsm_clock_force = value & (1 << 28) != 0;

        if value & (1 << 29) != 0 {
            self.state = UlpState::Reset;
            self.wake_remaining = None;
            return;
        }
        if self.state == UlpState::Reset {
            self.state = UlpState::Off;
        }
        if value & ((1 << 31) | (1 << 30)) != 0 {
            self.start();
        }
    }

    pub fn configure_cocpu(&mut self, value: u32) {
        self.architecture = if value & (1 << 23) != 0 {
            UlpArchitecture::Fsm
        } else {
            UlpArchitecture::RiscV
        };
        self.clock_gate = value & (1 << 27) != 0;
        self.cocpu_clock_force = value & 1 != 0;
        if self.architecture == UlpArchitecture::RiscV && value & (1 << 25) != 0 {
            self.halt();
        }
    }

    pub fn set_wake_period(&mut self, value: u32) {
        self.wake_period = (value >> 8) & 0x00ff_ffff;
        if self.timer_enabled && self.state != UlpState::Running {
            self.arm_timer();
        }
    }

    pub fn tick(&mut self, ticks: u64) {
        let Some(remaining) = self.wake_remaining else {
            return;
        };
        if ticks < u64::from(remaining) {
            self.wake_remaining = Some(remaining - ticks as u32);
        } else {
            self.wake_remaining = None;
            self.start();
        }
    }

    pub fn halt(&mut self) {
        if self.state != UlpState::Running {
            return;
        }
        self.state = UlpState::Halted;
        self.halts += 1;
        self.trace("halt");
        if self.timer_enabled {
            self.arm_timer();
        }
    }

    pub fn next_deadline(&self) -> Option<u64> {
        self.wake_remaining.map(u64::from)
    }

    pub fn debug(&mut self, on: bool) {
        self.debug = on;
    }

    pub fn debug_enabled(&self) -> bool {
        self.debug
    }

    pub fn report(&self) -> Option<String> {
        if self.starts == 0
            && self.halts == 0
            && self.wake_remaining.is_none()
            && self.state == UlpState::Off
        {
            return None;
        }
        Some(format!(
            "[emu] ulp: {:?} {:?}, entry {}, {} starts, {} halts",
            self.architecture, self.state, self.entry_pc, self.starts, self.halts
        ))
    }

    pub fn status_bits(&self) -> u32 {
        match self.state {
            UlpState::Halted => 1 << 16,
            UlpState::Off | UlpState::Reset => 1 << 15,
            UlpState::WakeDelay | UlpState::HaltDelay => 1 << 14,
            UlpState::Running => 1 << 13,
        }
    }

    fn arm_timer(&mut self) {
        let period = self.wake_period.max(1);
        self.wake_remaining = Some(period);
        self.state = UlpState::WakeDelay;
        self.trace("timer armed");
    }

    fn start(&mut self) {
        let clock_enabled = self.clock_gate
            && match self.architecture {
                UlpArchitecture::Fsm => self.fsm_clock_force,
                UlpArchitecture::RiscV => self.cocpu_clock_force,
            };
        if !clock_enabled || self.state == UlpState::Reset {
            return;
        }
        self.wake_remaining = None;
        self.state = UlpState::Running;
        self.starts += 1;
        self.trace("start");
    }

    fn trace(&self, event: &str) {
        if self.debug {
            eprintln!(
                "[ulp] {event}: {:?} {:?}, pc={}, starts={}, halts={}",
                self.architecture, self.state, self.entry_pc, self.starts, self.halts
            );
        }
    }
}

impl Default for UlpController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMER_EN: u32 = 1 << 31;
    const CLOCK_FORCE: u32 = 1 << 28;
    const RESET: u32 = 1 << 29;
    const FORCE_START: u32 = 1 << 30;
    const FSM_AND_CLOCK_GATE: u32 = (1 << 23) | (1 << 27);

    #[test]
    fn timer_start_uses_programmed_entry_and_deadline() {
        let mut ulp = UlpController::new();
        ulp.configure_cocpu(FSM_AND_CLOCK_GATE);
        ulp.configure_control(CLOCK_FORCE | (512 << 11) | 512);
        ulp.set_wake_period(25 << 8);
        ulp.configure_timer(TIMER_EN | 37);

        assert_eq!(ulp.entry_pc, 37);
        assert_eq!(ulp.state, UlpState::WakeDelay);
        assert_eq!(ulp.next_deadline(), Some(25));
        ulp.tick(24);
        assert_eq!(ulp.next_deadline(), Some(1));
        ulp.tick(1);
        assert_eq!(ulp.state, UlpState::Running);
        assert_eq!(ulp.starts, 1);
    }

    #[test]
    fn halt_rearms_an_enabled_periodic_timer() {
        let mut ulp = UlpController::new();
        ulp.configure_cocpu(FSM_AND_CLOCK_GATE);
        ulp.configure_control(CLOCK_FORCE | FORCE_START);
        ulp.configure_timer(TIMER_EN);
        assert_eq!(ulp.state, UlpState::WakeDelay);
        ulp.configure_control(CLOCK_FORCE | FORCE_START);
        assert_eq!(ulp.state, UlpState::Running);

        ulp.halt();
        assert_eq!(ulp.state, UlpState::WakeDelay);
        assert_eq!(ulp.halts, 1);
        assert_eq!(ulp.next_deadline(), Some(DEFAULT_WAKE_PERIOD as u64));
    }

    #[test]
    fn reset_blocks_force_start_until_released() {
        let mut ulp = UlpController::new();
        ulp.configure_cocpu(FSM_AND_CLOCK_GATE);
        ulp.configure_control(CLOCK_FORCE | RESET | FORCE_START);
        assert_eq!(ulp.state, UlpState::Reset);
        assert_eq!(ulp.starts, 0);

        ulp.configure_control(CLOCK_FORCE);
        assert_eq!(ulp.state, UlpState::Off);
        ulp.configure_control(CLOCK_FORCE | FORCE_START);
        assert_eq!(ulp.state, UlpState::Running);
    }

    #[test]
    fn cocpu_select_switches_architecture() {
        let mut ulp = UlpController::new();
        ulp.configure_cocpu(0);
        assert_eq!(ulp.architecture, UlpArchitecture::RiscV);
        ulp.configure_cocpu(1 << 23);
        assert_eq!(ulp.architecture, UlpArchitecture::Fsm);
    }
}
