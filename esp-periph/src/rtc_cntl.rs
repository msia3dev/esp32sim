use crate::device::{Device, WriteEffect};
use crate::regram::RegRam;
use crate::ulp::UlpController;
use emu_core::ClockDomain;

const INT_ENA: u32 = 0x40; const INT_RAW: u32 = 0x44; const INT_ST: u32 = 0x48; const INT_CLR: u32 = 0x4c;
const INT_ENA_W1TS: u32 = 0x138; const INT_ENA_W1TC: u32 = 0x13c; const INT_WDT: u32 = 1 << 3;
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

/// RTC_CNTL: reset control, slow-clock time, the RTC watchdog and the ULP controller.
/// WDTCONFIG0..WDTWPROTECT live at 0x98..0xb0 on S3 and 0x90..0xa8 on C3.
/// `esp_restart()` on ESP-IDF 5.x arms this watchdog and spins until it resets the chip.
pub struct RtcCntl { pub ram: RegRam, pub slow_ticks: u64, pub time_latch: u64, pub sw_reset: bool, pub reset_cause: u32, pub ulp: UlpController,
                     pub ulp_adc: [[u16; 16]; 2], pub ulp_tsens: u16, pub ulp_i2c: [[u8; 256]; 16], pub ulp_adc_sample_cycle: u8, pub ulp_adc_sample_bits: u8,
                     ulp_enabled: bool, wdt_base: u32, wdt_count: u64, wdt_stage: usize, wdt_unlocked: bool,
                     pub sens_adc: bool, pub analog: crate::analog::AnalogInputs, pub now_cycles: u64 }
impl RtcCntl {
    pub fn preset_after_bootloader(&mut self) { self.ram.write(0xc0, 0xFFD7_0028); self.ram.write(0xc4, 0xFF0F_00F0); }
    fn request_reset(&mut self, cause: u32) { if !self.sw_reset { self.sw_reset = true; self.reset_cause = cause; } }
    /// Advance the watchdog by RTC slow-clock ticks.
    pub fn wdt_tick(&mut self, ticks: u64) {
        let conf0 = self.ram.read(self.wdt_base);
        if conf0 & (1 << 31) == 0 { return; }
        self.wdt_count += ticks;
        while self.wdt_stage < 4 {
            let timeout = self.ram.read(self.wdt_base + 4 + 4 * self.wdt_stage as u32) as u64;
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
        let mut r = Self::with_wdt_base(0x98); r.sens_adc = true; r.ulp_enabled = true;
        // S3-only reset registers: +0x100/+0x104 are the classic chip's RTC memory CRC.
        r.init_ulp(); r
    }
    pub fn new_c3() -> Self { Self::with_wdt_base(0x90) }
    pub fn with_wdt_base(wdt_base: u32) -> Self {
        let mut r = RtcCntl { ram: RegRam::new(), slow_ticks: 0, time_latch: 0, sw_reset: false, reset_cause: RST_POWERON, ulp: UlpController::new(),
                              ulp_adc: [[0; 16]; 2], ulp_tsens: 128, ulp_i2c: [[0; 256]; 16], ulp_adc_sample_cycle: 2, ulp_adc_sample_bits: 12,
                              ulp_enabled: false, wdt_base, wdt_count: 0, wdt_stage: 0, wdt_unlocked: false, sens_adc: false, analog: crate::analog::AnalogInputs::new(240_000_000), now_cycles: 0 };
        r.ram.write(0x38, 1 | (1 << 6));           // RESET_STATE: reset cause POWERON for both CPUs
        r.ram.write(0x74, 0);                        // CLK_CONF
        r
    }
    fn init_ulp(&mut self) {
        self.ram.write(0x100, (512 << 11) | 512);       // ULP_CP_CTRL reset values
        self.ram.write(0x104, (1 << 23) | (40 << 14) | (16 << 7) | (8 << 1));
        self.ram.write(0x134, 200 << 8);                // ULP_CP_TIMER_1 reset value
        self.ram.write(0x818, (10 << 16) | 10); self.ram.write(0x81c, 10 << 16);
        self.ram.write(0x850, 6 << 14); self.ram.write(0xc00, 256); self.ram.write(0xc14, 256); self.ram.write(0xc1c, 8); self.ram.write(0xc20, 8);
    }
    pub fn read(&mut self, off: u32) -> u32 {
        match off {
            0x10 => self.time_latch as u32, 0x14 => (self.time_latch >> 32) as u32,
            0xc => self.ram.read(off) | (1 << 30),  // TIME_UPDATE: valid
            INT_ST if self.ulp_enabled => self.interrupt_status(),
            INT_CLR | INT_ENA_W1TS | INT_ENA_W1TC if self.ulp_enabled => 0,
            0xd0 if self.ulp_enabled => (self.ram.read(off) & !(0xf << 13)) | self.ulp.status_bits(),
            0x1fc => 0x2007270,
            0x850 => (self.ram.read(off) & !0x1ff) | (1 << 8) | u32::from(self.ulp_tsens & 0xff),
            _ => self.ram.read(off),
        }
    }
    pub fn write(&mut self, off: u32, v: u32) -> WriteEffect {
        let mut effect = WriteEffect::NONE;
        let wdt = self.wdt_base;
        match off {
            0x0 => { if v & (1 << 31) != 0 { self.request_reset(RST_SW_SYS); } else if v & (1 << 5) != 0 { self.request_reset(RST_SW_CPU); } self.ram.write(off, v & !((1 << 31) | (1 << 5))); }   // OPTIONS0.SW_SYS_RST / SW_PROCPU_RST
            0xc => { if v & (1 << 31) != 0 { self.time_latch = self.slow_ticks; } self.ram.write(off, v); }
            0x18 if self.ulp_enabled => { if v & 1 != 0 { self.raise_interrupt(INT_COCPU); } self.ram.write(off, v & !1); }
            INT_ENA | INT_RAW if self.ulp_enabled => self.ram.write(off, v),
            INT_ST if self.ulp_enabled => {}
            INT_CLR if self.ulp_enabled => self.ram.write(INT_RAW, self.ram.read(INT_RAW) & !v),
            INT_ENA_W1TS if self.ulp_enabled => self.ram.write(INT_ENA, self.ram.read(INT_ENA) | v),
            INT_ENA_W1TC if self.ulp_enabled => self.ram.write(INT_ENA, self.ram.read(INT_ENA) & !v),
            0x80c | 0x830 if self.sens_adc => self.sens_meas(off, v),
            _ if off == wdt + 0x18 => { self.wdt_unlocked = v == 0x50D8_3AA1; self.ram.write(off, v); }
            _ if (wdt..=wdt + 0x10).contains(&off) => { if self.wdt_unlocked { if off == wdt && (v ^ self.ram.read(wdt)) & (1 << 31) != 0 { self.wdt_count = 0; self.wdt_stage = 0; } self.ram.write(off, v); } }
            _ if off == wdt + 0x14 => { if self.wdt_unlocked && v & (1 << 31) != 0 { self.wdt_count = 0; self.wdt_stage = 0; } }   // WDTFEED
            0xfc if self.ulp_enabled => { self.ram.write(off, v); self.ulp.configure_timer(v); effect = WriteEffect::ULP; }
            0x100 if self.ulp_enabled => { self.ram.write(off, v); self.ulp.configure_control(v); effect = WriteEffect::ULP; }
            0x104 if self.ulp_enabled => { self.ram.write(off, v); self.ulp.configure_cocpu(v); effect = WriteEffect::ULP; }
            0x134 if self.ulp_enabled => { self.ram.write(off, v); self.ulp.set_wake_period(v); effect = WriteEffect::ULP; }
            0x404 if self.ulp_enabled => self.ram.write(0x400, self.ram.read(0x400) | v),
            0x408 if self.ulp_enabled => self.ram.write(0x400, self.ram.read(0x400) & !v),
            0x410 if self.ulp_enabled => self.ram.write(0x40c, self.ram.read(0x40c) | v),
            0x414 if self.ulp_enabled => self.ram.write(0x40c, self.ram.read(0x40c) & !v),
            _ => self.ram.write(off, v),
        }
        effect
    }

    pub fn raise_ulp_interrupt(&mut self) { self.raise_interrupt(INT_ULP_CP); }
    pub fn raise_cocpu_trap_interrupt(&mut self) { self.raise_interrupt(INT_COCPU_TRAP); }
    pub fn interrupt_status(&self) -> u32 { self.ram.read(INT_RAW) & self.ram.read(INT_ENA) }
    fn raise_interrupt(&mut self, mask: u32) { self.ram.write(INT_RAW, self.ram.read(INT_RAW) | mask); }
}

impl RtcCntl {
    /// The 12-bit ADC1 code a typical S3 gives for `volts` at `atten`: the inverse of the IDF 4.4
    /// calibration in `sar_adc` (the code whose calibrated reading is nearest), saturating at 4095, so
    /// analogReadMilliVolts() on the firmware reports the injected voltage to within a code.
    /// Noise, INL beyond Espressif's fitted curve and chip-to-chip spread are not modelled.
    pub fn s3_adc_code(volts: f32, atten: u32) -> u32 {
        crate::sar_adc::voltage_code(volts, atten, crate::sar_adc::Calibration::S3Adc1)
    }
    /// SENS_SAR_MEAS1_CTRL2 (IDF adc_ll_rtc_start_convert / _convert_is_done / _get_convert_value):
    /// EN_PAD [30:19] picks the ADC1 channel, START [17] rising runs one conversion, DONE [16]
    /// and DATA [15:0] carry the result. ADC1 channel n is GPIO n+1 on the S3; the channel's
    /// attenuation is SENS_SAR_ATTEN1 (+0x814) bits [2n+1:2n]. ADC2 uses CTRL2 +0x830,
    /// ATTEN2 +0x838 and GPIO n+11. Both convert instantaneously.
    fn sens_meas(&mut self, off: u32, v: u32) {
        let prev = self.ram.read(off);
        let out = crate::sar_adc::sens_oneshot(prev, v & !0xffff, || {
            let pads = (v >> 19) & 0xfff;
            let code = if pads == 0 { 0 } else {
                let ch = pads.trailing_zeros();
                let adc2 = off == 0x830;
                let atten = (self.ram.read(if adc2 { 0x838 } else { 0x814 }) >> (2 * ch)) & 3;
                if ch >= 10 { 0 } else {
                    self.analog.convert(ch as u8 + if adc2 { 11 } else { 1 }, self.now_cycles, |v| {
                        if adc2 { crate::sar_adc::voltage_code(v, atten, crate::sar_adc::Calibration::S3Adc2) }
                        else { Self::s3_adc_code(v, atten) }
                    })
                }
            };
            (1 << 16) | code
        });
        self.ram.write(off, out);
    }
}

impl Default for RtcCntl { fn default() -> Self { Self::new() } }

impl Device for RtcCntl {
    fn read(&mut self, off: u32) -> u32 { RtcCntl::read(self, off) }
    fn write(&mut self, off: u32, v: u32) -> WriteEffect { RtcCntl::write(self, off, v) }
    fn irq_sources(&self) -> u64 { u64::from(self.ulp_enabled && self.interrupt_status() != 0) }
    fn clock(&self) -> Option<ClockDomain> { Some(ClockDomain::RtcSlow) }
    fn tick(&mut self, ticks: u64) { self.slow_ticks += ticks; self.wdt_tick(ticks); if self.ulp_enabled { self.ulp.tick(ticks); } }
    fn has_deadline(&self) -> bool { self.ulp_enabled }
    fn next_deadline(&self) -> Option<u64> { if self.ulp_enabled { self.ulp.next_deadline() } else { None } }
    fn debug(&mut self, on: bool) { self.ulp.debug(on); }
}
