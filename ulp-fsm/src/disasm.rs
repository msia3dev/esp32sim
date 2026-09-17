//! Stable diagnostic formatting for decoded ULP-FSM instructions.

use crate::decode::{AluOp, BranchCond, Insn, Kind, StoreKind, StoreMode, Target};

fn r(reg: u8) -> String {
    format!("r{}", reg & 3)
}
fn alu(op: AluOp) -> &'static str {
    match op {
        AluOp::Add => "add",
        AluOp::Sub => "sub",
        AluOp::And => "and",
        AluOp::Or => "or",
        AluOp::Move => "move",
        AluOp::Lsh => "lsh",
        AluOp::Rsh => "rsh",
    }
}
fn cond(cond: BranchCond) -> &'static str {
    match cond {
        BranchCond::Always => "",
        BranchCond::Zero => "eq",
        BranchCond::Overflow => "ov",
        BranchCond::Lt => "lt",
        BranchCond::Gt => "gt",
        BranchCond::Eq => "eq",
        BranchCond::Ge => "ge",
        BranchCond::Le => "le",
    }
}

pub fn format(insn: &Insn) -> String {
    match insn.kind {
        Kind::Illegal => format!(".word 0x{:08x}", insn.raw),
        Kind::Wait { cycles } => format!("wait {cycles}"),
        Kind::Halt => "halt".into(),
        Kind::End { wake: true } => "wake".into(),
        Kind::End { wake: false } => "end".into(),
        Kind::Sleep { timer } => format!("sleep {timer}"),
        Kind::WriteReg {
            addr,
            peripheral,
            data,
            low,
            high,
        } => format!(
            "reg_wr {}, {high}, {low}, {data}",
            u16::from(addr) | (u16::from(peripheral) << 8)
        ),
        Kind::ReadReg {
            addr,
            peripheral,
            low,
            high,
        } => format!(
            "reg_rd {}, {high}, {low}",
            u16::from(addr) | (u16::from(peripheral) << 8)
        ),
        Kind::I2c {
            addr,
            data,
            low,
            high,
            bus,
            write: true,
        } => format!("i2c_wr 0x{addr:02x}, 0x{data:02x}, {high}, {low}, {bus}"),
        Kind::I2c {
            addr,
            low,
            high,
            bus,
            write: false,
            ..
        } => format!("i2c_rd 0x{addr:02x}, {high}, {low}, {bus}"),
        Kind::Adc { dest, sar, mux, .. } => {
            format!("adc {}, {}, {}", r(dest), sar, mux.saturating_sub(1))
        }
        Kind::Tsens { dest, delay } => format!("tsens {}, {delay}", r(dest)),
        Kind::Load {
            dest,
            addr,
            offset,
            upper: false,
        } => format!("ld {}, {}, {}", r(dest), r(addr), offset * 4),
        Kind::Load {
            dest,
            addr,
            offset,
            upper: true,
        } => format!("ldh {}, {}, {}", r(dest), r(addr), offset * 4),
        Kind::Store {
            kind: StoreKind::Offset,
            offset,
            ..
        } => format!("sto {}", offset * 4),
        Kind::Store {
            kind,
            mode,
            src,
            addr,
            offset,
            label,
            upper,
        } => {
            let mnemonic = match (kind, mode, upper) {
                (StoreKind::Manual, StoreMode::Full, _) => "st32",
                (StoreKind::Manual, StoreMode::Label, false) => "stl",
                (StoreKind::Manual, StoreMode::Label, true) => "sth",
                (StoreKind::Manual, StoreMode::Half, false) => "st",
                (StoreKind::Manual, StoreMode::Half, true) => "sth",
                (StoreKind::Auto, StoreMode::Full, _) => "sti32",
                (StoreKind::Auto, StoreMode::Label, _) => "sti",
                (StoreKind::Auto, StoreMode::Half, _) => "sti",
                (StoreKind::Offset, _, _) => unreachable!(),
            };
            match kind {
                StoreKind::Auto if matches!(mode, StoreMode::Half) => {
                    format!("{mnemonic} {}, {}", r(src), r(addr))
                }
                StoreKind::Auto => format!("{mnemonic} {}, {}, {label}", r(src), r(addr)),
                StoreKind::Manual if matches!(mode, StoreMode::Full | StoreMode::Label) => {
                    format!(
                        "{mnemonic} {}, {}, {}, {label}",
                        r(src),
                        r(addr),
                        offset * 4
                    )
                }
                StoreKind::Manual => format!("{mnemonic} {}, {}, {}", r(src), r(addr), offset * 4),
                StoreKind::Offset => unreachable!(),
            }
        }
        Kind::AluReg {
            op: AluOp::Move,
            dest,
            lhs,
            ..
        } => format!("move {}, {}", r(dest), r(lhs)),
        Kind::AluReg { op, dest, lhs, rhs } => {
            format!("{} {}, {}, {}", alu(op), r(dest), r(lhs), r(rhs))
        }
        Kind::AluImm {
            op: AluOp::Move,
            dest,
            imm,
            ..
        } => format!("move {}, 0x{imm:x}", r(dest)),
        Kind::AluImm { op, dest, src, imm } => {
            format!("{} {}, {}, {imm}", alu(op), r(dest), r(src))
        }
        Kind::Stage {
            op: AluOp::Add,
            imm,
        } => format!("stage_inc {imm}"),
        Kind::Stage {
            op: AluOp::Sub,
            imm,
        } => format!("stage_dec {imm}"),
        Kind::Stage {
            op: AluOp::Move, ..
        } => "stage_rst".into(),
        Kind::Stage { .. } => unreachable!(),
        Kind::BranchAbs {
            target,
            cond: BranchCond::Always,
        } => match target {
            Target::Immediate(pc) => format!("jump {}", pc * 4),
            Target::Register(reg) => format!("jump {}", r(reg)),
        },
        Kind::BranchAbs { target, cond: c } => match target {
            Target::Immediate(pc) => format!("jump {}, {}", pc * 4, cond(c)),
            Target::Register(reg) => format!("jump {}, {}", r(reg), cond(c)),
        },
        Kind::BranchRel {
            offset,
            imm,
            cond: c,
            stage,
        } => format!(
            "{} {}, {}, {}",
            if stage { "jumps" } else { "jumpr" },
            offset * 4,
            imm,
            cond(c)
        ),
    }
}
