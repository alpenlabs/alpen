use borsh::{BorshDeserialize, BorshSerialize};
use strata_msg_fmt::TypeId;

use crate::{
    asm_stf::AsmStfUpdate, checkpoint::CheckpointUpdate, deposit::DepositLog,
    export::NewExportEntry, forced_inclusion::ForcedInclusionData,
};

/// Container for any structured ASM log emitted by the system.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "TODO: should we refactor to avoid large enum variants"
)]
pub enum AuxLog {
    Deposit(DepositLog),
    ForcedInclusion(ForcedInclusionData),
    Checkpoint(CheckpointUpdate),
    AsmStf(AsmStfUpdate),
    NewExport(NewExportEntry),
}

/// Trait for ASM log types that can be serialized and stored.
///
/// This trait provides a consistent interface for log entries that need to be
/// serialized, stored, and later deserialized from the ASM state. Each log type
/// has a unique type identifier and must be serializable.
// TODO migrate from borsh for this
pub trait AsmLog: BorshSerialize + BorshDeserialize {
    /// Unique type identifier for this log type.
    ///
    /// This constant is used to distinguish between different log types when
    /// serializing and deserializing log entries.
    const TY: TypeId;
}
