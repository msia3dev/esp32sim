//! Decoder for the 32-bit ESP32-S2/S3 ULP-FSM instruction encoding.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AluOp {
    Add,
    Sub,
    And,
    Or,
    Move,
    Lsh,
    Rsh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchCond {
    Always,
    Zero,
    Overflow,
    Lt,
    Gt,
    Eq,
    Ge,
    Le,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Immediate(u16),
    Register(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    Manual,
    Auto,
    Offset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreMode {
    Full,
    Label,
    Half,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Illegal,
    Wait {
        cycles: u16,
    },
    Halt,
    End {
        wake: bool,
    },
    Sleep {
        timer: u8,
    },
    WriteReg {
        addr: u8,
        peripheral: u8,
        data: u8,
        low: u8,
        high: u8,
    },
    ReadReg {
        addr: u8,
        peripheral: u8,
        low: u8,
        high: u8,
    },
    I2c {
        addr: u8,
        data: u8,
        low: u8,
        high: u8,
        bus: u8,
        write: bool,
    },
    Adc {
        dest: u8,
        sar: u8,
        mux: u8,
        cycles: u16,
    },
    Tsens {
        dest: u8,
        delay: u16,
    },
    Store {
        kind: StoreKind,
        mode: StoreMode,
        src: u8,
        addr: u8,
        offset: i16,
        label: u8,
        upper: bool,
    },
    Load {
        dest: u8,
        addr: u8,
        offset: i16,
        upper: bool,
    },
    AluReg {
        op: AluOp,
        dest: u8,
        lhs: u8,
        rhs: u8,
    },
    AluImm {
        op: AluOp,
        dest: u8,
        src: u8,
        imm: u16,
    },
    Stage {
        op: AluOp,
        imm: u8,
    },
    BranchAbs {
        target: Target,
        cond: BranchCond,
    },
    BranchRel {
        offset: i16,
        imm: u16,
        cond: BranchCond,
        stage: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effects(u8);
impl Effects {
    pub const NONE: Self = Self(0);
    pub const CONTROL_FLOW: Self = Self(1);
    pub const MEMORY: Self = Self(2);
    pub const MMIO: Self = Self(4);
    pub const WAKE: Self = Self(8);
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}
impl std::ops::BitOr for Effects {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insn {
    pub raw: u32,
    pub kind: Kind,
}
impl Insn {
    pub fn is_illegal(self) -> bool {
        self.kind == Kind::Illegal
    }
    pub fn effects(self) -> Effects {
        match self.kind {
            Kind::Illegal
            | Kind::Wait { .. }
            | Kind::AluReg { .. }
            | Kind::AluImm { .. }
            | Kind::Stage { .. } => Effects::NONE,
            Kind::Halt | Kind::Sleep { .. } | Kind::BranchAbs { .. } | Kind::BranchRel { .. } => {
                Effects::CONTROL_FLOW
            }
            Kind::End { wake: false } => Effects::MMIO,
            Kind::End { wake: true } => Effects::WAKE,
            Kind::WriteReg { .. }
            | Kind::ReadReg { .. }
            | Kind::I2c { .. }
            | Kind::Adc { .. }
            | Kind::Tsens { .. } => Effects::MMIO,
            Kind::Store { .. } | Kind::Load { .. } => Effects::MEMORY,
        }
    }

    /// Total ULP clock cycles, including the documented fetch cost.
    /// Peripheral-dependent instructions return `None` until their U4 timing inputs are modeled.
    pub fn cycles(self) -> Option<u32> {
        Some(match self.kind {
            Kind::Illegal | Kind::I2c { .. } | Kind::Adc { .. } | Kind::Tsens { .. } => {
                return None
            }
            Kind::Halt => 2,
            Kind::BranchAbs { .. } | Kind::BranchRel { .. } => 4,
            Kind::Store { .. } | Kind::Load { .. } | Kind::ReadReg { .. } => 8,
            Kind::WriteReg { .. } => 12,
            Kind::Wait { cycles } => 6 + u32::from(cycles),
            Kind::End { .. }
            | Kind::Sleep { .. }
            | Kind::AluReg { .. }
            | Kind::AluImm { .. }
            | Kind::Stage { .. } => 6,
        })
    }
}

fn alu(sel: u32) -> Option<AluOp> {
    Some(match sel {
        0 => AluOp::Add,
        1 => AluOp::Sub,
        2 => AluOp::And,
        3 => AluOp::Or,
        4 => AluOp::Move,
        5 => AluOp::Lsh,
        6 => AluOp::Rsh,
        _ => return None,
    })
}
fn illegal(raw: u32) -> Insn {
    Insn {
        raw,
        kind: Kind::Illegal,
    }
}
fn reg(raw: u32, shift: u32) -> u8 {
    ((raw >> shift) & 3) as u8
}
fn signed11(raw: u32, shift: u32) -> i16 {
    let value = ((raw >> shift) & 0x7ff) as i16;
    (value << 5) >> 5
}

pub fn decode(raw: u32) -> Insn {
    let opcode = raw >> 28;
    let kind = match opcode {
        1 => {
            let low = ((raw >> 18) & 0x1f) as u8;
            let high = ((raw >> 23) & 0x1f) as u8;
            if low > high {
                return illegal(raw);
            }
            Kind::WriteReg {
                addr: raw as u8,
                peripheral: ((raw >> 8) & 3) as u8,
                data: ((raw >> 10) & 0xff) as u8,
                low,
                high,
            }
        }
        2 => {
            let low = ((raw >> 18) & 0x1f) as u8;
            let high = ((raw >> 23) & 0x1f) as u8;
            if raw & 0x0003_fc00 != 0 || low > high {
                return illegal(raw);
            }
            Kind::ReadReg {
                addr: raw as u8,
                peripheral: ((raw >> 8) & 3) as u8,
                low,
                high,
            }
        }
        3 => Kind::I2c {
            addr: raw as u8,
            data: (raw >> 8) as u8,
            low: ((raw >> 16) & 7) as u8,
            high: ((raw >> 19) & 7) as u8,
            bus: ((raw >> 22) & 0xf) as u8,
            write: raw & (1 << 27) != 0,
        },
        4 if raw & 0x0fff_0000 == 0 => Kind::Wait { cycles: raw as u16 },
        5 if raw & 0x0f00_0080 == 0 => {
            let mux = ((raw >> 2) & 0xf) as u8;
            if mux == 0 {
                return illegal(raw);
            }
            Kind::Adc {
                dest: reg(raw, 0),
                mux,
                sar: ((raw >> 6) & 1) as u8,
                cycles: ((raw >> 8) & 0xffff) as u16,
            }
        }
        6 => {
            if raw & 0x01e0_0200 != 0 {
                return illegal(raw);
            }
            let kind = match (raw >> 25) & 7 {
                1 => StoreKind::Auto,
                3 => StoreKind::Offset,
                4 => StoreKind::Manual,
                _ => return illegal(raw),
            };
            let mode = match (raw >> 7) & 3 {
                0 => StoreMode::Full,
                1 => StoreMode::Label,
                3 => StoreMode::Half,
                _ => return illegal(raw),
            };
            Kind::Store {
                kind,
                mode,
                src: reg(raw, 0),
                addr: reg(raw, 2),
                label: ((raw >> 4) & 3) as u8,
                upper: raw & (1 << 6) != 0,
                offset: signed11(raw, 10),
            }
        }
        7 => match (raw >> 26) & 3 {
            0 if raw & 0x021f_ffc0 == 0 => Kind::AluReg {
                op: match alu((raw >> 21) & 0xf) {
                    Some(v) => v,
                    None => return illegal(raw),
                },
                dest: reg(raw, 0),
                lhs: reg(raw, 2),
                rhs: reg(raw, 4),
            },
            1 if raw & 0x0210_0000 == 0 => Kind::AluImm {
                op: match alu((raw >> 21) & 0xf) {
                    Some(v) => v,
                    None => return illegal(raw),
                },
                dest: reg(raw, 0),
                src: reg(raw, 2),
                imm: ((raw >> 4) & 0xffff) as u16,
            },
            2 if raw & 0x021f_f00f == 0 => {
                let op = match (raw >> 21) & 0xf {
                    0 => AluOp::Add,
                    1 => AluOp::Sub,
                    2 => AluOp::Move,
                    _ => return illegal(raw),
                };
                Kind::Stage {
                    op,
                    imm: ((raw >> 4) & 0xff) as u8,
                }
            }
            _ => return illegal(raw),
        },
        8 => match (raw >> 26) & 3 {
            1 if raw & 0x021f_e000 == 0 => {
                let cond = match (raw >> 22) & 7 {
                    0 => BranchCond::Always,
                    1 => BranchCond::Zero,
                    2 => BranchCond::Overflow,
                    _ => return illegal(raw),
                };
                let target = if raw & (1 << 21) != 0 {
                    Target::Register(reg(raw, 0))
                } else {
                    Target::Immediate(((raw >> 2) & 0x7ff) as u16)
                };
                Kind::BranchAbs { target, cond }
            }
            sub @ (0 | 2) => {
                let cmp = (raw >> 16) & 3;
                let cond = match (sub, cmp) {
                    (0, 0) => BranchCond::Lt,
                    (0, 1) => BranchCond::Gt,
                    (0, 2) => BranchCond::Eq,
                    (2, _) => match (((raw >> 15) & 1) << 2) | cmp {
                        2 => BranchCond::Eq,
                        4 => BranchCond::Lt,
                        5 => BranchCond::Gt,
                        6 => BranchCond::Le,
                        7 => BranchCond::Ge,
                        _ => return illegal(raw),
                    },
                    _ => return illegal(raw),
                };
                if sub == 2 && raw & 0x0000_7f00 != 0 {
                    return illegal(raw);
                }
                let magnitude = ((raw >> 18) & 0x7f) as i16;
                Kind::BranchRel {
                    offset: if raw & (1 << 25) != 0 {
                        -magnitude
                    } else {
                        magnitude
                    },
                    imm: if sub == 2 {
                        (raw & 0xff) as u16
                    } else {
                        raw as u16
                    },
                    cond,
                    stage: sub == 2,
                }
            }
            _ => return illegal(raw),
        },
        9 if raw & 0x03ff_fffe == 0 => match (raw >> 26) & 3 {
            0 => Kind::End { wake: raw & 1 != 0 },
            1 => Kind::Sleep {
                timer: (raw & 1) as u8,
            },
            _ => return illegal(raw),
        },
        10 if raw & 0x0fff_0000 == 0 => Kind::Tsens {
            dest: reg(raw, 0),
            delay: ((raw >> 2) & 0x3fff) as u16,
        },
        11 if raw & 0x0fff_ffff == 0 => Kind::Halt,
        13 if raw & 0x07e0_03f0 == 0 => Kind::Load {
            dest: reg(raw, 0),
            addr: reg(raw, 2),
            offset: signed11(raw, 10),
            upper: raw & (1 << 27) != 0,
        },
        _ => return illegal(raw),
    };
    Insn { raw, kind }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_and_unknown_encodings_are_illegal() {
        for raw in [
            0,
            0xc000_0000,
            0xf000_0000,
            0xb000_0001,
            0x4001_0000,
            0x2000_0400,
            0x7600_0000,
        ] {
            assert!(decode(raw).is_illegal(), "{raw:08x}");
        }
    }

    #[test]
    fn effect_metadata_separates_memory_mmio_and_control_flow() {
        assert!(decode(0x6800_0d8e).effects().contains(Effects::MEMORY));
        assert!(decode(0x1184_1404).effects().contains(Effects::MMIO));
        assert!(decode(0x8400_000c)
            .effects()
            .contains(Effects::CONTROL_FLOW));
        assert!(decode(0x9000_0001).effects().contains(Effects::WAKE));
    }
}
