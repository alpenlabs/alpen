use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_msg_fmt::TypeId;
use strata_primitives::{l1::L1BlockCommitment, l2::L2BlockCommitment};

use crate::constants::CHECKPOINT_UPDATE_LOG_TYPE;

/// Details for a checkpoint update event.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
// TODO: Append "Log" to the name like other
pub struct CheckpointUpdate {
    /// L1 block commitment reference.
    pub l1_ref: L1BlockCommitment,
    /// Verified L2 block commitment reference.
    pub verified_blk: L2BlockCommitment,
}

impl CheckpointUpdate {
    /// Create a new CheckpointUpdate instance.
    pub fn new(l1_ref: L1BlockCommitment, verified_blk: L2BlockCommitment) -> Self {
        Self {
            l1_ref,
            verified_blk,
        }
    }
}

impl AsmLog for CheckpointUpdate {
    const TY: TypeId = CHECKPOINT_UPDATE_LOG_TYPE;
}
