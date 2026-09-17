//! Boards around the SoC. The SoC model emits generic events (GPIO edges, RMT symbol streams,
//! SPI bytes, LCD frames, camera requests); a `BoardModel` interprets them as the devices wired to
//! the pins and offers what the UI and the scripts need back.
use esp_periph::i2c::I2cDevice;

pub type VirtualCycle = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoardEdge {
    pub cycle: VirtualCycle,
    pub pin: u8,
    pub level: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanelControl {
    pub name: &'static str,
    pub label: &'static str,
    pub x: u16,
    pub y: u16,
}

/// Convert one timestamped digital GPIO level into deterministic mono PCM.
///
/// The fixed-point high-pass stage models the AC-coupled response of a small transducer: PWM
/// remains audible while a steady high or low level decays to silence instead of becoming a DC
/// audio offset. Boards choose the pin; this type only consumes its executed waveform.
pub struct GpioAudio {
    cpu_hz: u64,
    sample_rate: u32,
    amplitude: i32,
    cycle: VirtualCycle,
    level: bool,
    input: i32,
    output: i32,
    edges: u64,
    samples: Vec<i16>,
}

impl GpioAudio {
    pub fn new(cpu_hz: u64, sample_rate: u32, amplitude: i16) -> Self {
        assert!(cpu_hz > 0 && sample_rate > 0);
        Self { cpu_hz, sample_rate, amplitude: i32::from(amplitude), cycle: 0, level: false, input: 0, output: 0, edges: 0, samples: Vec::new() }
    }

    pub fn set_level(&mut self, cycle: VirtualCycle, level: bool) {
        self.advance_to(cycle);
        if self.level != level { self.level = level; self.edges += 1; }
    }

    pub fn advance_to(&mut self, cycle: VirtualCycle) {
        assert!(cycle >= self.cycle, "GPIO audio time moved backwards");
        let target = ((u128::from(cycle) * u128::from(self.sample_rate)) / u128::from(self.cpu_hz)) as usize;
        while self.samples.len() < target {
            let input = if self.level { self.amplitude } else { 0 };
            // alpha = 32600/32768: about a 36 Hz high-pass corner at 44.1 kHz.
            let product = self.output * 32600;
            let feedback = if product >= 0 { product >> 15 } else { -((-product) >> 15) };
            self.output = input - self.input + feedback;
            self.input = input;
            self.samples.push(self.output.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16);
        }
        self.cycle = cycle;
    }

    pub fn audio(&self) -> (&[i16], u32) { (&self.samples, self.sample_rate) }
    pub fn edges(&self) -> u64 { self.edges }
}

/// What a board does with the SoC's pin-level activity.
pub trait BoardModel {
    fn name(&self) -> &'static str;
    /// GPIO output level changes, in order.
    fn gpio_changes(&mut self, _changes: &[(u8, bool)]) {}
    /// Timestamped form used by SoCs which preserve exact output-edge completion times.
    fn gpio_changes_at(&mut self, _cycle: VirtualCycle, changes: &[(u8, bool)]) { self.gpio_changes(changes); }
    /// A completed RMT transmission, decoded to bits by the peripheral model, with the pin the
    /// GPIO matrix has that channel routed to. Drivers that take a fresh channel per refresh
    /// (the Arduino NeoPixel one does) make the channel meaningless; the pin names the strip.
    fn rmt_frame(&mut self, _pin: u8, _bits: &[bool]) {}
    /// Bytes a GP-SPI master (`host` = 2 or 3) shifted out on MOSI.
    fn spi_tx(&mut self, _host: u8, _data: &[u8]) {}
    /// One complete GP-SPI transaction. The default preserves transmit-only boards and models an
    /// unattached MISO line.
    fn spi_transfer(&mut self, host: u8, tx: &[u8], rx_len: usize) -> Vec<u8> {
        self.spi_tx(host, tx);
        vec![0xff; rx_len]
    }
    fn gpio_events(&self) -> u64 { 0 }
    /// Devices on the I2C buses: (bus, 7-bit address, device).
    fn i2c_devices(&mut self) -> Vec<(u8, u8, Box<dyn I2cDevice>)> { Vec::new() }
    /// Give the board's camera a picture to look at (RGB888).
    fn set_camera_picture(&mut self, _p: crate::picture::Picture) {}
    /// Next camera frame as the sensor would put it on the DVP bus (YUYV), with its size. None = no camera / nothing to show.
    fn camera_frame(&mut self) -> Option<(u32, u32, std::sync::Arc<Vec<u8>>)> { None }
    /// Small RGB preview of what the camera is looking at (for the UI), if a picture is loaded.
    fn camera_preview(&self, _w: u32, _h: u32) -> Option<Vec<u8>> { None }
    /// A complete frame from the LCD_CAM RGB interface (RGB565 little-endian, `w`x`h`).
    fn lcd_frame(&mut self, _w: u32, _h: u32, _rgb565: &[u8]) {}
    /// The board's display for the UI/PNG: (width, height, RGB565 pixels, change counter).
    fn display(&self) -> Option<(u32, u32, Vec<u16>, u64)> { None }
    /// Preferred clockwise presentation rotation for the dashboard. Raw framebuffer/PNG data
    /// remains in controller coordinates; a URL `?rotate=` selection overrides this hint.
    fn display_rotation(&self) -> u16 { 0 }
    /// Optional controls rendered directly below the dashboard display. Coordinates are in raw
    /// panel space and are delivered through the same touch path as a framebuffer pointer event.
    fn panel_controls(&self) -> Vec<PanelControl> { Vec::new() }
    /// Completed display frames (for the UI's statistics line).
    fn display_frames(&self) -> u64 { 0 }
    /// Optional board-generated mono PCM, for GPIO transducers and other non-I2S audio paths.
    fn audio(&self) -> Option<(&[i16], u32)> { None }
    /// Cheap change counter of the display (`display().3` without building the frame).
    fn display_version(&self) -> u64 { 0 }
    /// Prefer waiting one push interval for a quiet pixel stream. The UI still publishes on
    /// the next opportunity during continuous changes so animation cannot starve.
    fn display_quiet_push(&self) -> bool { false }
    /// Raw display memory for a debug PNG: (pixels, columns, rows).
    fn gram(&self) -> Option<(Vec<u16>, usize, usize)> { None }
    /// LED ring / strip: colours and a change counter.
    fn leds(&self) -> Option<(&[[u8; 3]], u64)> { None }
    /// Addressable LED modules besides `leds()`, each with the port it sits in and a change
    /// counter: (id, colours, updates). The UI draws one square grid per entry.
    fn led_grids(&self) -> Vec<(&'static str, &[[u8; 3]], u64)> { Vec::new() }
    /// Touch input from the UI (panel coordinates).
    fn touch(&mut self, _x: u16, _y: u16, _down: bool) {}
    /// Touch input observed at a specific bus horizon. Untimed boards use the ordinary input path.
    fn touch_at(&mut self, _cycle: VirtualCycle, x: u16, y: u16, down: bool) { self.touch(x, y, down); }
    /// Current board-driven GPIO input levels, used to reconnect a persistent board after reset.
    fn input_levels(&self) -> Vec<(u8, bool)> { Vec::new() }
    /// Earliest autonomous transition strictly after the board's current cycle.
    fn next_deadline(&self) -> Option<VirtualCycle> { None }
    /// Advance monotonically through every board transition due by `cycle`.
    fn advance_to(&mut self, _cycle: VirtualCycle) {}
    /// Timestamped GPIO input edges emitted by the last advance.
    fn take_edges(&mut self) -> Vec<BoardEdge> { Vec::new() }
    /// A pin by the name scripts and the UI use (`btn1`, `sw`, ...).
    fn named_pin(&self, _name: &str) -> Option<u8> { None }
    /// The rotary encoder's (CLK, DT) pins, if there is one.
    fn encoder(&self) -> Option<(u8, u8)> { None }
    /// Lines for the end-of-run report.
    fn report(&self) -> String { String::new() }
}

pub type Board = Box<dyn BoardModel>;

/// A bare module: nothing on the pins, console only.
pub struct NoBoard;
impl BoardModel for NoBoard { fn name(&self) -> &'static str { "none" } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpio_audio_preserves_edge_sample_boundaries_and_decays_dc() {
        let mut audio = GpioAudio::new(1_000, 100, 12_000);
        audio.set_level(10, true);
        audio.set_level(20, false);
        audio.advance_to(200);
        let (samples, rate) = audio.audio();
        assert_eq!((samples.len(), rate, audio.edges()), (20, 100, 2));
        assert_eq!(samples[0], 0, "the high edge occurs after the first sample boundary");
        assert!(samples[1] > 0 && samples[2] < 0, "the two waveform edges have opposite polarity");
        assert!(samples.last().unwrap().abs() < samples[2].abs(), "a steady low level decays toward silence");
    }
}
