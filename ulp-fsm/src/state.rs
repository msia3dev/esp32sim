//! Architectural state of the ESP32-S2/S3 ULP-FSM core.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cpu {
    pub regs: [u16; 4],
    /// Program counter in 32-bit RTC slow-memory words.
    pub pc: u16,
    pub stage: u8,
    pub zero: bool,
    pub overflow: bool,
    pub halted: bool,
    pub insn_count: u64,
    pub cycle_count: u64,
    pub store_offset: i16,
    pub store_upper_next: bool,
}

impl Cpu {
    pub fn new(entry_pc: u16) -> Self {
        Self {
            regs: [0; 4],
            pc: entry_pc & 0x7ff,
            stage: 0,
            zero: false,
            overflow: false,
            halted: false,
            insn_count: 0,
            cycle_count: 0,
            store_offset: 0,
            store_upper_next: false,
        }
    }

    pub fn restart(&mut self, entry_pc: u16) {
        self.pc = entry_pc & 0x7ff;
        self.halted = false;
    }

    pub fn reset_architecture(&mut self, entry_pc: u16) {
        self.regs = [0; 4];
        self.pc = entry_pc & 0x7ff;
        self.stage = 0;
        self.zero = false;
        self.overflow = false;
        self.halted = false;
        self.store_offset = 0;
        self.store_upper_next = false;
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new(0)
    }
}
