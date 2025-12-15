//! SSZ-compatible checkpoint update log type.
//!
//! This module provides the `CheckpointUpdateSsz` log type for the checkpoint subprotocol,
//! using the SPS-62 SSZ-based checkpoint types.

// TODO: cleanup
// Rename `CheckpointUpdateSsz` to `CheckpointUpdate` and remove the
// `CHECKPOINT_UPDATE_SSZ_LOG_TYPE` constant when the legacy `CheckpointUpdate` (in
// `checkpoint.rs`) is removed after csm-worker deprecation.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_checkpoint_types_ssz::{BatchInfo, BatchTransition, CheckpointPayload};
use strata_codec::Codec;
use strata_codec_utils::CodecBorsh;
use strata_identifiers::EpochCommitment;
use strata_msg_fmt::TypeId;
use strata_primitives::l1::BitcoinTxid;

use crate::constants::CHECKPOINT_UPDATE_SSZ_LOG_TYPE;

/// Checkpoint update log details using SSZ types.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Codec)]
pub struct CheckpointUpdateSsz {
    /// Commitment to the epoch terminal block.
    epoch_commitment: EpochCommitment,

    /// Metadata describing the checkpoint batch.
    batch_info: CodecBorsh<BatchInfo>,

    /// State transition committed by the checkpoint proof.
    transition: CodecBorsh<BatchTransition>,

    /// Hash of the L1 transaction that carried the checkpoint proof.
    checkpoint_txid: CodecBorsh<BitcoinTxid>,
}

impl CheckpointUpdateSsz {
    /// Create a new CheckpointUpdateSsz instance.
    pub fn new(
        epoch_commitment: EpochCommitment,
        batch_info: BatchInfo,
        transition: BatchTransition,
        checkpoint_txid: BitcoinTxid,
    ) -> Self {
        Self {
            epoch_commitment,
            batch_info: CodecBorsh::new(batch_info),
            transition: CodecBorsh::new(transition),
            checkpoint_txid: CodecBorsh::new(checkpoint_txid),
        }
    }

    /// Construct a `CheckpointUpdateSsz` from a `CheckpointPayload`.
    pub fn from_payload(payload: &CheckpointPayload, checkpoint_txid: BitcoinTxid) -> Self {
        let batch_info = &payload.commitment.batch_info;
        let transition = &payload.commitment.transition;

        // Construct epoch commitment from epoch and terminal L2 block
        let epoch_commitment =
            EpochCommitment::from_terminal(batch_info.epoch, batch_info.l2_range.end);

        Self::new(epoch_commitment, *batch_info, *transition, checkpoint_txid)
    }

    /// Returns the epoch commitment.
    pub fn epoch_commitment(&self) -> EpochCommitment {
        self.epoch_commitment
    }

    /// Returns the batch info.
    pub fn batch_info(&self) -> &BatchInfo {
        self.batch_info.inner()
    }

    /// Returns the state transition.
    pub fn transition(&self) -> &BatchTransition {
        self.transition.inner()
    }

    /// Returns the checkpoint transaction ID.
    pub fn checkpoint_txid(&self) -> &BitcoinTxid {
        self.checkpoint_txid.inner()
    }
}

impl AsmLog for CheckpointUpdateSsz {
    const TY: TypeId = CHECKPOINT_UPDATE_SSZ_LOG_TYPE;
}
