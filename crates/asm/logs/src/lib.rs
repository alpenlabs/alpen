//! # ASM Log Types
//!
//! This crate provides structured log types for the Anchor State Machine (ASM) in the Strata
//! protocol. It defines various log entry types that capture important events within the system.

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
use strata_asm_common::AsmLogEntry;

use crate::constants::{CHECKPOINT_UPDATE_LOG_TYPE, DEPOSIT_LOG_TYPE_ID};

#[derive(Clone, Debug)]
#[expect(clippy::large_enum_variant, reason = "..")]
pub enum ParsedAsmLog {
    Checkpoint(CheckpointUpdate),
    Deposit(DepositLog),
}

impl TryFrom<AsmLogEntry> for ParsedAsmLog {
    type Error = AsmParseError;

    fn try_from(log: AsmLogEntry) -> Result<Self, Self::Error> {
        match log.ty() {
            Some(CHECKPOINT_UPDATE_LOG_TYPE) => log
                .try_into_log::<CheckpointUpdate>()
                .map(Self::Checkpoint)
                .map_err(|_| AsmParseError::InvalidLogData),

            Some(DEPOSIT_LOG_TYPE_ID) => log
                .try_into_log::<DepositLog>()
                .map(Self::Deposit)
                .map_err(|_| AsmParseError::InvalidLogData),
            Some(_) | None => Err(AsmParseError::UnknownLogType),
        }
    }
}

/// Error type for parsing ASM log entries.
#[derive(Clone, Debug)]
pub enum AsmParseError {
    /// The log type identifier is not recognized.
    UnknownLogType,
    /// The log data could not be parsed into the expected format.
    InvalidLogData,
}

impl std::fmt::Display for AsmParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownLogType => write!(f, "unknown log type"),
            Self::InvalidLogData => write!(f, "invalid log data"),
        }
    }
}

impl std::error::Error for AsmParseError {}
