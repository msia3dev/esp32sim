//! ESP32-S2/S3 ULP-FSM architecture core.
//!
//! The decoder is derived from ESP-IDF 5.2.1's public ESP32-S3 `ulp_insn_t` layouts. SoC
//! lifecycle and RTC peripheral ownership remain outside this crate.
pub mod decode;
pub mod disasm;
pub mod exec;
pub mod state;

pub use decode::{decode, AluOp, BranchCond, Effects, Insn, Kind, StoreKind, StoreMode, Target};
pub use exec::{execute, step, Bus, Event, Trap};
pub use state::Cpu;
