//! # ASM Log Types
//!
//! This crate provides structured log types for the Anchor State Machine (ASM) in the Strata
//! protocol. It defines various log entry types that capture important events within the system.

use borsh::{BorshDeserialize, BorshSerialize};

pub mod asm_stf;
pub mod checkpoint;
pub mod constants;
pub mod deposit;
pub mod export;
pub mod forced_inclusion;

pub use asm_stf::AsmStfUpdate;
pub use checkpoint::CheckpointUpdate;
pub use deposit::DepositLog;
pub use export::NewExportEntry;
pub use forced_inclusion::ForcedInclusionData;

/// Enum wrapping all supported ASM log types.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum AsmLogType {
    AsmStfUpdate(AsmStfUpdate),
    CheckpointUpdate(CheckpointUpdate),
    DepositLog(DepositLog),
    NewExportEntry(NewExportEntry),
    ForcedInclusionData(ForcedInclusionData),
}
