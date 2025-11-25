use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_checkpoint_types::{BatchInfo, ChainstateRootTransition, Checkpoint};
use strata_codec::Codec;
use strata_codec_utils::CodecBorsh;
use strata_msg_fmt::TypeId;
use strata_primitives::{epoch::EpochCommitment, l1::BitcoinTxid};

use crate::constants::CHECKPOINT_UPDATE_LOG_TYPE;

/// Details for a checkpoint update event.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Codec)]
pub struct CheckpointUpdate {
    /// Commitment to the epoch terminal block.
    epoch_commitment: EpochCommitment,

    /// Metadata describing the checkpoint batch.
    batch_info: CodecBorsh<BatchInfo>,

    /// Chainstate transition committed by the checkpoint proof.
    chainstate_transition: CodecBorsh<ChainstateRootTransition>,

    /// Hash of the L1 transaction that carried the checkpoint proof.
    checkpoint_txid: CodecBorsh<BitcoinTxid>,
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
            batch_info: CodecBorsh::new(batch_info),
            chainstate_transition: CodecBorsh::new(chainstate_transition),
            checkpoint_txid: CodecBorsh::new(checkpoint_txid),
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

    pub fn epoch_commitment(&self) -> EpochCommitment {
        self.epoch_commitment
    }

    pub fn batch_info(&self) -> &BatchInfo {
        self.batch_info.inner()
    }

    pub fn chainstate_transition(&self) -> &ChainstateRootTransition {
        self.chainstate_transition.inner()
    }

    pub fn checkpoint_txid(&self) -> &BitcoinTxid {
        self.checkpoint_txid.inner()
    }
}

impl AsmLog for CheckpointUpdate {
    const TY: TypeId = CHECKPOINT_UPDATE_LOG_TYPE;
}
