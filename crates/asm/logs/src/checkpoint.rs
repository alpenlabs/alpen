use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_msg_fmt::TypeId;
use strata_primitives::{epoch::EpochCommitment, l1::L1BlockCommitment};

use crate::constants::CHECKPOINT_UPDATE_LOG_TYPE;

/// Details for a checkpoint update event.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct CheckpointUpdateLog {
    /// L1 block commitment reference.
    pub l1_ref: L1BlockCommitment,
    /// Epoch Commitment
    pub epoch_commitment: EpochCommitment,
}

impl CheckpointUpdateLog {
    /// Create a new CheckpointUpdate instance.
    pub fn new(l1_ref: L1BlockCommitment, epoch_commitment: EpochCommitment) -> Self {
        Self {
            l1_ref,
            epoch_commitment,
        }
    }
}

impl AsmLog for CheckpointUpdateLog {
    const TY: TypeId = CHECKPOINT_UPDATE_LOG_TYPE;
}
