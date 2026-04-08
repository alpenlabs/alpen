//! Alpen-local v0 checkpoint log types.
//!
//! The v0 checkpoint subprotocol is legacy. It emits [`CheckpointUpdate`],
//! which carries full checkpoint metadata (epoch commitment, batch info, L1
//! txid). The main (v1) checkpoint subprotocol in the asm repo has its own
//! `CheckpointTipUpdate` log type and superseded this one.
//!
//! Because v0 is still in service, the log type stays in alpen alongside the
//! v0 subprotocol. Once v0 is retired, this module can be deleted.

use strata_asm_common::AsmLog;
use strata_btc_types::BitcoinTxid;
use strata_checkpoint_types::{BatchInfo, Checkpoint};
use strata_codec::Codec;
use strata_codec_utils::CodecSsz;
use strata_identifiers::EpochCommitment;
use strata_msg_fmt::TypeId;

/// Log type id for [`CheckpointUpdate`]. Historically `3` — preserved so v0
/// checkpoint logs on chain continue to parse.
pub const CHECKPOINT_UPDATE_LOG_TYPE: TypeId = 3;

/// V0 checkpoint log emitted by the v0 checkpoint subprotocol.
///
/// Carries full checkpoint metadata (batch info, chainstate transition
/// commitment, L1 transaction id). Superseded by `CheckpointTipUpdate` in the
/// v1 checkpoint subprotocol from the asm repo.
#[derive(Debug, Clone, Codec)]
pub struct CheckpointUpdate {
    /// Commitment to the epoch terminal block.
    epoch_commitment: CodecSsz<EpochCommitment>,

    /// Metadata describing the checkpoint batch.
    batch_info: CodecSsz<BatchInfo>,

    /// Hash of the L1 transaction that carried the checkpoint proof.
    checkpoint_txid: CodecSsz<BitcoinTxid>,
}

impl CheckpointUpdate {
    /// Creates a new [`CheckpointUpdate`] log entry.
    pub fn new(
        epoch_commitment: EpochCommitment,
        batch_info: BatchInfo,
        checkpoint_txid: BitcoinTxid,
    ) -> Self {
        Self {
            epoch_commitment: CodecSsz::new(epoch_commitment),
            batch_info: CodecSsz::new(batch_info),
            checkpoint_txid: CodecSsz::new(checkpoint_txid),
        }
    }

    /// Constructs a [`CheckpointUpdate`] from a verified checkpoint instance.
    pub fn from_checkpoint(checkpoint: &Checkpoint, checkpoint_txid: BitcoinTxid) -> Self {
        let batch_info = checkpoint.batch_info();
        Self::new(
            batch_info.get_epoch_commitment(),
            batch_info.clone(),
            checkpoint_txid,
        )
    }

    /// Returns the epoch commitment this update corresponds to.
    pub fn epoch_commitment(&self) -> EpochCommitment {
        *self.epoch_commitment.inner()
    }

    /// Returns the batch info associated with the update.
    pub fn batch_info(&self) -> &BatchInfo {
        self.batch_info.inner()
    }

    /// Returns the txid of the L1 transaction carrying the checkpoint proof.
    pub fn checkpoint_txid(&self) -> &BitcoinTxid {
        self.checkpoint_txid.inner()
    }
}

impl AsmLog for CheckpointUpdate {
    const TY: TypeId = CHECKPOINT_UPDATE_LOG_TYPE;
}
