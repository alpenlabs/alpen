use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_msg_fmt::TypeId;
use strata_primitives::{
    batch::{BatchInfo, ChainstateRootTransition, Checkpoint},
    epoch::EpochCommitment,
    l1::BitcoinTxid,
};

use crate::constants::CHECKPOINT_UPDATE_LOG_TYPE;

/// Details for a checkpoint update event.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct CheckpointUpdate {
    /// Commitment to the epoch terminal block.
    pub epoch_commitment: EpochCommitment,
    /// Metadata describing the checkpoint batch.
    pub batch_info: BatchInfo,
    /// Chainstate transition committed by the checkpoint proof.
    pub chainstate_transition: ChainstateRootTransition,
    /// Hash of the L1 transaction that carried the checkpoint proof.
    pub checkpoint_txid: BitcoinTxid,
}

impl CheckpointUpdate {
    /// Create a new CheckpointUpdate instance.
    pub fn new(
        epoch_commitment: EpochCommitment,
        batch_info: BatchInfo,
        chainstate_transition: ChainstateRootTransition,
        checkpoint_txid: BitcoinTxid,
    ) -> Self {
        Self {
            epoch_commitment,
            batch_info,
            chainstate_transition,
            checkpoint_txid,
        }
    }

    /// Construct a `CheckpointUpdate` from a verified checkpoint instance.
    pub fn from_checkpoint(checkpoint: &Checkpoint, checkpoint_txid: BitcoinTxid) -> Self {
        let batch_info = checkpoint.batch_info();
        let transition = checkpoint.batch_transition();
        let chainstate_transition = transition.chainstate_transition;

        Self::new(
            batch_info.get_epoch_commitment(),
            batch_info.clone(),
            chainstate_transition,
            checkpoint_txid,
        )
    }
}

impl AsmLog for CheckpointUpdate {
    const TY: TypeId = CHECKPOINT_UPDATE_LOG_TYPE;
}
