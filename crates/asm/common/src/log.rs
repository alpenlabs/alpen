use moho_types::{ExportEntry, InnerVerificationKey};
use strata_primitives::{l1::L1BlockCommitment, l2::L2BlockCommitment, proof::RollupVerifyingKey};

/// Enumeration of ASM log events with their associated data.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum AsmLog {
    /// Deposit event carrying deposit operation details.
    Deposit(DepositLog),
    /// Forced inclusion event carrying raw payload details.
    ForcedInclusion(ForcedInclusionData),
    /// Checkpoint update event carrying block commitment references.
    CheckpointUpdate(CheckpointUpdate),
    /// Verification key update for the rollup state transition function.
    OlStfUpdate(OlStfUpdate),
    /// Verification key update for the execution environment state transition.
    AsmStfUpdate(AsmStfUpdate),
    /// Export state update event carrying export entry data.
    NewExportEntry(NewExportEntry),
}

/// Details for a deposit operation.
#[derive(Debug, Clone)]
pub struct DepositLog {
    /// Identifier of the target execution environment.
    pub ee_id: u64,
    /// Amount in satoshis.
    pub amount: u64,
    /// Serialized address for the operation.
    pub addr: Vec<u8>,
}

/// Details for a forced inclusion operation.
#[derive(Debug, Clone)]
pub struct ForcedInclusionData {
    /// Identifier of the target execution environment.
    pub ee_id: u64,
    /// Raw payload data for inclusion.
    pub payload: Vec<u8>,
}

/// Details for a checkpoint update event.
#[derive(Debug, Clone)]
pub struct CheckpointUpdate {
    /// L1 block commitment reference.
    pub l1_ref: L1BlockCommitment,
    /// Verified L2 block commitment reference.
    pub verified_blk: L2BlockCommitment,
}

/// Details for a rollup verification key update.
#[derive(Debug, Clone)]
pub struct OlStfUpdate {
    /// New rollup state transition function verification key.
    pub new_vk: RollupVerifyingKey,
}

/// Details for an execution environment verification key update.
#[derive(Debug, Clone)]
pub struct AsmStfUpdate {
    /// New execution environment state transition function verification key.
    pub new_vk: InnerVerificationKey,
}

/// Details for an export state update event.
#[derive(Debug, Clone)]
pub struct NewExportEntry {
    /// Export entry data.
    pub entry: ExportEntry,
}
