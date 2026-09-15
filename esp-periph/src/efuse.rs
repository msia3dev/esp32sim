use crate::device::{Device, WriteEffect};
use crate::regram::RegRam;

// ------------------------------------------------------------------ efuse
pub struct Efuse { pub ram: RegRam }
impl Efuse {
    pub fn new(mac: [u8; 6]) -> Self {
        let mut e = Efuse { ram: RegRam::new() };
        e.ram.write(0x44, u32::from_be_bytes([mac[2], mac[3], mac[4], mac[5]]));
        e.ram.write(0x48, ((mac[0] as u32) << 8 | mac[1] as u32) | (2 << 18));   // wafer_version_minor_lo = 2 (rev v0.2)
        e.ram.write(0x6c, 1);                                                    // blk_version_major = 1
        e.ram.write(0x1cc, 0x8c);
        e.ram.write(0x1d0, 0);
        e
    }
    pub fn read(&self, off: u32) -> u32 { self.ram.read(off) }
    pub fn write(&mut self, off: u32, v: u32) {
        match off {
            0x1d4 => {                                                     // CMD: read/pgm done immediately
                if v & 2 != 0 { self.program((v >> 2) & 0xf); }
            }
            _ => self.ram.write(off, v)
        }
    }

    fn program(&mut self, block: u32) {
        let (start, words) = match block {
            0 => (0x2c, 6),
            1 => (0x44, 6),
            2 => (0x5c, 8),
            3..=10 => (0x7c + (block - 3) * 0x20, 8),
            _ => return,
        };
        for word in 0..words {
            let staged = self.ram.read(word * 4);
            let off = start + word * 4;
            self.ram.write(off, self.ram.read(off) | staged);              // eFuse bits are one-way
        }
    }
}

impl Device for Efuse {
    fn read(&mut self, off: u32) -> u32 { Efuse::read(self, off) }
    fn write(&mut self, off: u32, v: u32) -> WriteEffect { Efuse::write(self, off, v); WriteEffect::NONE }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_command_burns_staged_words_into_the_selected_block() {
        let mut efuse = Efuse::new([0; 6]);
        efuse.write(0, 0x0000_0003);
        efuse.write(4, 0x1122_3344);

        efuse.write(0x1d4, (4 << 2) | 2);

        assert_eq!(efuse.read(0x9c), 0x0000_0003);
        assert_eq!(efuse.read(0xa0), 0x1122_3344);
        assert_eq!(efuse.read(0xbc), 0);
        assert_eq!(efuse.read(0x1d4), 0);
    }

    #[test]
    fn programming_can_only_set_additional_bits() {
        let mut efuse = Efuse::new([0; 6]);
        efuse.ram.write(0x9c, 0x0000_0001);
        efuse.write(0, 0x0000_0002);
        efuse.write(0x1d4, (4 << 2) | 2);

        assert_eq!(efuse.read(0x9c), 0x0000_0003);
    }
}
