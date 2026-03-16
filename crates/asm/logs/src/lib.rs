//! # ASM Log Types
//!
//! This crate provides structured log types for the Anchor State Machine (ASM) in the Strata
//! protocol. It defines various log entry types that capture important events within the system.

#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub mod asm_stf;
pub mod checkpoint;
pub mod constants;
pub mod deposit;
pub mod export;
pub mod forced_inclusion;

pub use asm_stf::AsmStfUpdate;
pub use checkpoint::{CheckpointTipUpdate, CheckpointUpdate};
pub use deposit::DepositLog;
pub use export::NewExportEntry;
pub use forced_inclusion::ForcedInclusionData;
