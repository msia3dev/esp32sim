//! One-instruction reference interpreter for the ESP32-S2/S3 ULP-FSM core.

use crate::decode::{decode, AluOp, BranchCond, Kind, StoreKind, StoreMode, Target};
use crate::state::Cpu;

pub trait Bus {
    type Error;
    fn read_word(&mut self, address: u16) -> Result<u32, Self::Error>;
    fn write_word(&mut self, address: u16, value: u32) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Continue,
    Halt,
    Wake,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trap<E> {
    Illegal { pc: u16, raw: u32 },
    Bus(E),
    Peripheral,
    UndefinedShift(u16),
}

fn address(base: u16, offset: i16) -> u16 {
    base.wrapping_add(offset as u16) & 0x7ff
}

fn compare(value: u16, threshold: u16, cond: BranchCond) -> bool {
    match cond {
        BranchCond::Lt => value < threshold,
        BranchCond::Gt => value > threshold,
        BranchCond::Eq => value == threshold,
        BranchCond::Ge => value >= threshold,
        BranchCond::Le => value <= threshold,
        _ => false,
    }
}

fn alu(cpu: &mut Cpu, op: AluOp, lhs: u16, rhs: u16) -> Result<u16, u16> {
    let prior_overflow = cpu.overflow;
    let (result, overflow) = match op {
        AluOp::Add => lhs.overflowing_add(rhs),
        AluOp::Sub => lhs.overflowing_sub(rhs),
        AluOp::And => (lhs & rhs, prior_overflow),
        AluOp::Or => (lhs | rhs, prior_overflow),
        AluOp::Move => (rhs, prior_overflow),
        AluOp::Lsh if rhs <= 15 => (lhs << rhs, prior_overflow),
        AluOp::Rsh if rhs <= 15 => (lhs >> rhs, prior_overflow),
        AluOp::Lsh | AluOp::Rsh => return Err(rhs),
    };
    cpu.zero = result == 0;
    cpu.overflow = overflow;
    Ok(result)
}

pub fn step<B: Bus>(cpu: &mut Cpu, bus: &mut B) -> Result<Event, Trap<B::Error>> {
    if cpu.halted {
        return Ok(Event::Halt);
    }
    let pc = cpu.pc;
    let raw = bus.read_word(pc).map_err(Trap::Bus)?;
    let insn = decode(raw);
    if insn.is_illegal() {
        return Err(Trap::Illegal { pc, raw });
    }
    cpu.pc = cpu.pc.wrapping_add(1) & 0x7ff;
    let mut cycles = 6u64;
    let event = match insn.kind {
        Kind::Illegal => unreachable!("illegal instructions return before execution"),
        Kind::Wait { cycles: wait } => {
            cycles += u64::from(wait);
            Event::Continue
        }
        Kind::Halt => {
            cycles = 2;
            cpu.halted = true;
            Event::Halt
        }
        Kind::End { wake: true } => Event::Wake,
        Kind::AluReg { op, dest, lhs, rhs } => {
            let value = alu(cpu, op, cpu.regs[lhs as usize], cpu.regs[rhs as usize])
                .map_err(Trap::UndefinedShift)?;
            cpu.regs[dest as usize] = value;
            Event::Continue
        }
        Kind::AluImm { op, dest, src, imm } => {
            let value = alu(cpu, op, cpu.regs[src as usize], imm).map_err(Trap::UndefinedShift)?;
            cpu.regs[dest as usize] = value;
            Event::Continue
        }
        Kind::Stage {
            op: AluOp::Add,
            imm,
        } => {
            cpu.stage = cpu.stage.wrapping_add(imm);
            Event::Continue
        }
        Kind::Stage {
            op: AluOp::Sub,
            imm,
        } => {
            cpu.stage = cpu.stage.wrapping_sub(imm);
            Event::Continue
        }
        Kind::Stage {
            op: AluOp::Move, ..
        } => {
            cpu.stage = 0;
            Event::Continue
        }
        Kind::Stage { .. } => unreachable!(),
        Kind::Load {
            dest,
            addr,
            offset,
            upper,
        } => {
            cycles = 8;
            let word = bus
                .read_word(address(cpu.regs[addr as usize], offset))
                .map_err(Trap::Bus)?;
            cpu.regs[dest as usize] = if upper {
                (word >> 16) as u16
            } else {
                word as u16
            };
            Event::Continue
        }
        Kind::Store {
            kind: StoreKind::Offset,
            offset,
            ..
        } => {
            cycles = 8;
            cpu.store_offset = offset;
            cpu.store_upper_next = false;
            Event::Continue
        }
        Kind::Store {
            kind,
            mode,
            src,
            addr,
            offset,
            label,
            upper,
        } => {
            cycles = 8;
            let auto = kind == StoreKind::Auto;
            let target = address(
                cpu.regs[addr as usize],
                if auto { cpu.store_offset } else { offset },
            );
            let old = bus.read_word(target).map_err(Trap::Bus)?;
            let value = cpu.regs[src as usize];
            let labelled = (u32::from(label) << 14) | u32::from(value & 0x3fff);
            let full = (u32::from(pc & 0x7ff) << 21) | (u32::from(label) << 16) | u32::from(value);
            let write_upper = if auto && mode != StoreMode::Full {
                cpu.store_upper_next
            } else {
                upper
            };
            let new = match mode {
                StoreMode::Full => full,
                StoreMode::Label if write_upper => (old & 0x0000_ffff) | (labelled << 16),
                StoreMode::Label => (old & 0xffff_0000) | labelled,
                StoreMode::Half if write_upper => (old & 0x0000_ffff) | (u32::from(value) << 16),
                StoreMode::Half => (old & 0xffff_0000) | u32::from(value),
            };
            bus.write_word(target, new).map_err(Trap::Bus)?;
            if auto {
                if mode == StoreMode::Full || cpu.store_upper_next {
                    cpu.store_offset = cpu.store_offset.wrapping_add(1);
                }
                if mode != StoreMode::Full {
                    cpu.store_upper_next = !cpu.store_upper_next;
                }
            }
            Event::Continue
        }
        Kind::BranchAbs { target, cond } => {
            cycles = 4;
            let taken = match cond {
                BranchCond::Always => true,
                BranchCond::Zero => cpu.zero,
                BranchCond::Overflow => cpu.overflow,
                _ => false,
            };
            if taken {
                cpu.pc = match target {
                    Target::Immediate(pc) => pc,
                    Target::Register(reg) => cpu.regs[reg as usize],
                } & 0x7ff;
            }
            Event::Continue
        }
        Kind::BranchRel {
            offset,
            imm,
            cond,
            stage,
        } => {
            cycles = 4;
            let value = if stage {
                u16::from(cpu.stage)
            } else {
                cpu.regs[0]
            };
            if compare(value, imm, cond) {
                cpu.pc = pc.wrapping_add(offset as u16) & 0x7ff;
            }
            Event::Continue
        }
        Kind::End { wake: false }
        | Kind::Sleep { .. }
        | Kind::WriteReg { .. }
        | Kind::ReadReg { .. }
        | Kind::I2c { .. }
        | Kind::Adc { .. }
        | Kind::Tsens { .. } => return Err(Trap::Peripheral),
    };
    cpu.insn_count += 1;
    cpu.cycle_count += cycles;
    Ok(event)
}
