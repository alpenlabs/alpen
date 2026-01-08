//! SSZ-compatible checkpoint update log type.
//!
//! This module provides the `CheckpointUpdate` log type for the checkpoint subprotocol,
//! using the SPS-62 SSZ-based checkpoint types.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmLog;
use strata_checkpoint_types_ssz::{BatchInfo, BatchTransition, CheckpointPayload};
use strata_codec::Codec;
use strata_codec_utils::CodecBorsh;
use strata_identifiers::EpochCommitment;
use strata_msg_fmt::TypeId;
use strata_primitives::l1::BitcoinTxid;

use crate::constants::CHECKPOINT_UPDATE_LOG_TYPE;

/// Checkpoint update log
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Codec)]
pub struct CheckpointUpdate {
    /// Commitment to the epoch terminal block.
    epoch_commitment: EpochCommitment,

    /// Metadata describing the checkpoint batch.
    batch_info: CodecBorsh<BatchInfo>,

    /// State transition committed by the checkpoint proof.
    transition: CodecBorsh<BatchTransition>,

    /// Hash of the L1 transaction that carried the checkpoint proof.
    checkpoint_txid: CodecBorsh<BitcoinTxid>,
}

impl CheckpointUpdate {
    /// Create a new CheckpointUpdate instance.
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

    /// Construct a `CheckpointUpdate` from a `CheckpointPayload` and pre-state root.
    ///
    /// The `pre_state_root` comes from ASM state (not the payload) since SPS-62
    /// only stores post-state in the on-chain payload to save L1 cost.
    pub fn from_payload(
        payload: &CheckpointPayload,
        pre_state_root: strata_identifiers::Buf32,
        checkpoint_txid: BitcoinTxid,
    ) -> Self {
        let batch_info = &payload.commitment.batch_info;
        let post_state_root = payload.commitment.post_state_root;
        let transition = BatchTransition::new(pre_state_root, post_state_root);

        // Construct epoch commitment from epoch and terminal L2 block
        let epoch_commitment =
            EpochCommitment::from_terminal(batch_info.epoch, batch_info.l2_range.end);

        Self::new(epoch_commitment, *batch_info, transition, checkpoint_txid)
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

impl AsmLog for CheckpointUpdate {
    const TY: TypeId = CHECKPOINT_UPDATE_LOG_TYPE;
}
