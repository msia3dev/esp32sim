use crate::device::{Device, WriteEffect};
use crate::regram::RegRam;
use crate::ulp::UlpController;
use emu_core::ClockDomain;

const INT_ENA: u32 = 0x40;
const INT_RAW: u32 = 0x44;
const INT_ST: u32 = 0x48;
const INT_CLR: u32 = 0x4c;
const INT_ENA_W1TS: u32 = 0x138;
const INT_ENA_W1TC: u32 = 0x13c;
const INT_WDT: u32 = 1 << 3;
pub const INT_ULP_CP: u32 = 1 << 5;
pub const INT_COCPU: u32 = 1 << 13;
pub const INT_COCPU_TRAP: u32 = 1 << 17;

// ------------------------------------------------------------------ RTC controller
/// Reset causes (RTC_CNTL_RESET_CAUSE_PROCPU), as the ROM prints them.
pub const RST_POWERON: u32 = 1; pub const RST_SW_SYS: u32 = 3; pub const RST_RTCWDT_SYS: u32 = 9; pub const RST_SW_CPU: u32 = 12;
pub const RST_RTCWDT_CPU: u32 = 13; pub const RST_RTCWDT_RTC: u32 = 16;
pub fn reset_cause_name(c: u32) -> &'static str {
    match c { 1 => "POWERON", 3 => "RTC_SW_SYS_RESET", 5 => "DEEPSLEEP", 7 => "TG0WDT_SYS_RESET", 8 => "TG1WDT_SYS_RESET", 9 => "RTCWDT_SYS_RESET", 11 => "TG0WDT_CPU_RESET",
            12 => "RTC_SW_CPU_RESET", 13 => "RTCWDT_CPU_RESET", 15 => "RTCWDT_BROWN_OUT_RESET", 16 => "RTCWDT_RTC_RESET", 17 => "TG1WDT_CPU_RESET", 18 => "SUPER_WDT_RESET", _ => "?" }
}

/// RTC_CNTL: reset control, slow-clock time, the RTC watchdog, and the ULP controller registers.
/// `esp_restart()` on ESP-IDF 5.x arms this watchdog and spins until it resets the chip.
pub struct RtcCntl { pub ram: RegRam, pub slow_ticks: u64, pub time_latch: u64, pub sw_reset: bool, pub reset_cause: u32, pub ulp: UlpController,
                     pub ulp_adc: [[u16; 16]; 2], pub ulp_tsens: u16, pub ulp_i2c: [[u8; 256]; 16], pub ulp_adc_sample_cycle: u8, pub ulp_adc_sample_bits: u8,
                     wdt_count: u64, wdt_stage: usize, wdt_unlocked: bool }
impl RtcCntl {
    pub fn preset_after_bootloader(&mut self) { self.ram.write(0xc0, 0xFFD7_0028); self.ram.write(0xc4, 0xFF0F_00F0); }
    fn request_reset(&mut self, cause: u32) { if !self.sw_reset { self.sw_reset = true; self.reset_cause = cause; } }
    /// Advance the watchdog by RTC slow-clock ticks.
    pub fn wdt_tick(&mut self, ticks: u64) {
        let conf0 = self.ram.read(0x98);
        if conf0 & (1 << 31) == 0 { return; }
        self.wdt_count += ticks;
        while self.wdt_stage < 4 {
            let timeout = self.ram.read(0x9c + 4 * self.wdt_stage as u32) as u64;
            let action = (conf0 >> (28 - 3 * self.wdt_stage as u32)) & 7;
            if action == 0 { self.wdt_stage += 1; continue; }              // stage disabled: skip
            if self.wdt_count < timeout { break; }
            self.wdt_count = 0; self.wdt_stage += 1;
            match action {
                1 => self.raise_interrupt(INT_WDT),
                2 => self.request_reset(RST_RTCWDT_CPU),
                3 => self.request_reset(RST_RTCWDT_SYS),
                4 => self.request_reset(RST_RTCWDT_RTC),
                _ => {}
            }
            if self.sw_reset { break; }
        }
        if self.wdt_stage >= 4 { self.wdt_stage = 0; }
    }
    pub fn new() -> Self {
        let mut r = RtcCntl { ram: RegRam::new(), slow_ticks: 0, time_latch: 0, sw_reset: false, reset_cause: RST_POWERON, ulp: UlpController::new(),
                              ulp_adc: [[0; 16]; 2], ulp_tsens: 128, ulp_i2c: [[0; 256]; 16], ulp_adc_sample_cycle: 2, ulp_adc_sample_bits: 12,
                              wdt_count: 0, wdt_stage: 0, wdt_unlocked: false };
        r.ram.write(0x38, 1 | (1 << 6));           // RESET_STATE: reset cause POWERON for both CPUs
        r.ram.write(0x74, 0);                        // CLK_CONF
        r.ram.write(0x100, (512 << 11) | 512);       // ULP_CP_CTRL reset values
        r.ram.write(0x104, (1 << 23) | (40 << 14) | (16 << 7) | (8 << 1));
        r.ram.write(0x134, 200 << 8);                // ULP_CP_TIMER_1 reset value
        r.ram.write(0x818, (10 << 16) | 10);          // SENS_SAR_AMP_CTRL1 reset values
        r.ram.write(0x81c, 10 << 16);                 // SENS_SAR_AMP_CTRL2 reset value
        r.ram.write(0x850, 6 << 14);                  // SENS_SAR_TSENS_CTRL clock divider
        r.ram.write(0xc00, 256); r.ram.write(0xc14, 256); // RTC_I2C SCL low/high
        r.ram.write(0xc1c, 8); r.ram.write(0xc20, 8);     // RTC_I2C start/stop
        r
    }
    pub fn read(&mut self, off: u32) -> u32 {
        match off {
            0x10 => self.time_latch as u32, 0x14 => (self.time_latch >> 32) as u32,
            0xc => self.ram.read(off) | (1 << 30),  // TIME_UPDATE: valid
            INT_ST => self.interrupt_status(),
            INT_CLR | INT_ENA_W1TS | INT_ENA_W1TC => 0,
            0xd0 => (self.ram.read(off) & !(0xf << 13)) | self.ulp.status_bits(),
            0x1fc => 0x2007270,
            0x850 => (self.ram.read(off) & !0x1ff) | (1 << 8) | u32::from(self.ulp_tsens & 0xff),
            _ => self.ram.read(off),
        }
    }
    pub fn write(&mut self, off: u32, v: u32) -> WriteEffect {
        let mut effect = WriteEffect::NONE;
        match off {
            0x0 => { if v & (1 << 31) != 0 { self.request_reset(RST_SW_SYS); } else if v & (1 << 5) != 0 { self.request_reset(RST_SW_CPU); } self.ram.write(off, v & !((1 << 31) | (1 << 5))); }   // OPTIONS0.SW_SYS_RST / SW_PROCPU_RST
            0xc => { if v & (1 << 31) != 0 { self.time_latch = self.slow_ticks; } self.ram.write(off, v); }
            0x18 => { if v & 1 != 0 { self.raise_interrupt(INT_COCPU); } self.ram.write(off, v & !1); }
            INT_ENA | INT_RAW => self.ram.write(off, v),
            INT_ST => {}
            INT_CLR => self.ram.write(INT_RAW, self.ram.read(INT_RAW) & !v),
            INT_ENA_W1TS => self.ram.write(INT_ENA, self.ram.read(INT_ENA) | v),
            INT_ENA_W1TC => self.ram.write(INT_ENA, self.ram.read(INT_ENA) & !v),
            0xb0 => { self.wdt_unlocked = v == 0x50D8_3AA1; self.ram.write(off, v); }
            0x98..=0xa8 => { if self.wdt_unlocked { if off == 0x98 && (v ^ self.ram.read(0x98)) & (1 << 31) != 0 { self.wdt_count = 0; self.wdt_stage = 0; } self.ram.write(off, v); } }
            0xac => { if self.wdt_unlocked && v & (1 << 31) != 0 { self.wdt_count = 0; self.wdt_stage = 0; } }   // WDTFEED
            0xfc => { self.ram.write(off, v); self.ulp.configure_timer(v); effect = WriteEffect::ULP; }
            0x100 => { self.ram.write(off, v); self.ulp.configure_control(v); effect = WriteEffect::ULP; }
            0x104 => { self.ram.write(off, v); self.ulp.configure_cocpu(v); effect = WriteEffect::ULP; }
            0x134 => { self.ram.write(off, v); self.ulp.set_wake_period(v); effect = WriteEffect::ULP; }
            0x404 => self.ram.write(0x400, self.ram.read(0x400) | v),       // RTC_GPIO_OUT_W1TS
            0x408 => self.ram.write(0x400, self.ram.read(0x400) & !v),      // RTC_GPIO_OUT_W1TC
            0x410 => self.ram.write(0x40c, self.ram.read(0x40c) | v),       // RTC_GPIO_ENABLE_W1TS
            0x414 => self.ram.write(0x40c, self.ram.read(0x40c) & !v),      // RTC_GPIO_ENABLE_W1TC
            _ => self.ram.write(off, v),
        }
        effect
    }

    pub fn raise_ulp_interrupt(&mut self) { self.raise_interrupt(INT_ULP_CP); }
    pub fn raise_cocpu_trap_interrupt(&mut self) { self.raise_interrupt(INT_COCPU_TRAP); }
    pub fn interrupt_status(&self) -> u32 { self.ram.read(INT_RAW) & self.ram.read(INT_ENA) }
    fn raise_interrupt(&mut self, mask: u32) { self.ram.write(INT_RAW, self.ram.read(INT_RAW) | mask); }
}

impl Default for RtcCntl { fn default() -> Self { Self::new() } }

impl Device for RtcCntl {
    fn read(&mut self, off: u32) -> u32 { RtcCntl::read(self, off) }
    fn write(&mut self, off: u32, v: u32) -> WriteEffect { RtcCntl::write(self, off, v) }
    fn irq_sources(&self) -> u64 { u64::from(self.interrupt_status() != 0) }
    fn clock(&self) -> Option<ClockDomain> { Some(ClockDomain::RtcSlow) }
    fn tick(&mut self, ticks: u64) { self.slow_ticks += ticks; self.wdt_tick(ticks); self.ulp.tick(ticks); }
    fn has_deadline(&self) -> bool { true }
    fn next_deadline(&self) -> Option<u64> { self.ulp.next_deadline() }
    fn debug(&mut self, on: bool) { self.ulp.debug(on); }
}
