use borsh::{BorshDeserialize, BorshSerialize};
use strata_msg_fmt::TypeId;
use strata_primitives::{l1::L1BlockCommitment, l2::L2BlockCommitment};

use crate::logs::{AsmLog, constants::CHECKPOINT_UPDATE_LOG_TYPE};

/// Details for a checkpoint update event.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct CheckpointUpdate {
    /// L1 block commitment reference.
    pub l1_ref: L1BlockCommitment,
    /// Verified L2 block commitment reference.
    pub verified_blk: L2BlockCommitment,
}

impl AsmLog for CheckpointUpdate {
    const TY: TypeId = CHECKPOINT_UPDATE_LOG_TYPE;
}
