//! Legacy checkpoint update log type.
//!
//! This module provides the `CheckpointUpdateLegacy` log type for backward compatibility
//! with csm-worker and other legacy code that uses the old checkpoint types.
//!
//! # TODO(cleanup)
//!
//! This entire module should be removed when csm-worker is deprecated after OL STF migration.
//! At that point, only the new `CheckpointUpdate` type (using SPS-62 types) should be used.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_checkpoint_types::{BatchInfo, ChainstateRootTransition, Checkpoint};
use strata_codec::Codec;
use strata_codec_utils::CodecBorsh;
use strata_msg_fmt::TypeId;
use strata_primitives::{epoch::EpochCommitment, l1::BitcoinTxid};

use crate::constants::CHECKPOINT_UPDATE_LEGACY_LOG_TYPE;

/// Legacy checkpoint update log details (uses old checkpoint types).
///
/// This struct uses the legacy `BatchInfo` and `ChainstateRootTransition` types
/// from `checkpoint-types` for compatibility with csm-worker.
///
/// # TODO(cleanup)
///
/// Remove this type when csm-worker is deprecated. Use `CheckpointUpdate` instead.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Codec)]
pub struct CheckpointUpdateLegacy {
    /// Commitment to the epoch terminal block.
    epoch_commitment: EpochCommitment,

    /// Metadata describing the checkpoint batch.
    batch_info: CodecBorsh<BatchInfo>,

    /// Chainstate root transition committed by the checkpoint proof.
    chainstate_transition: CodecBorsh<ChainstateRootTransition>,

    /// Hash of the L1 transaction that carried the checkpoint proof.
    checkpoint_txid: CodecBorsh<BitcoinTxid>,
}

impl CheckpointUpdateLegacy {
    /// Create a new CheckpointUpdateLegacy instance.
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

    /// Construct a `CheckpointUpdateLegacy` from a legacy `Checkpoint`.
    pub fn from_checkpoint(checkpoint: &Checkpoint, checkpoint_txid: BitcoinTxid) -> Self {
        let batch_info = checkpoint.batch_info();
        let chainstate_transition = checkpoint.batch_transition().chainstate_transition;

        Self::new(
            batch_info.get_epoch_commitment(),
            batch_info.clone(),
            chainstate_transition,
            checkpoint_txid,
        )
    }

    /// Returns the epoch commitment.
    pub fn epoch_commitment(&self) -> EpochCommitment {
        self.epoch_commitment
    }

    /// Returns the batch info.
    pub fn batch_info(&self) -> &BatchInfo {
        self.batch_info.inner()
    }

    /// Returns the chainstate root transition.
    pub fn chainstate_transition(&self) -> &ChainstateRootTransition {
        self.chainstate_transition.inner()
    }

    /// Returns the checkpoint transaction ID.
    pub fn checkpoint_txid(&self) -> &BitcoinTxid {
        self.checkpoint_txid.inner()
    }
}

impl AsmLog for CheckpointUpdateLegacy {
    const TY: TypeId = CHECKPOINT_UPDATE_LEGACY_LOG_TYPE;
}
