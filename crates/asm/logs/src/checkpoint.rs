use strata_asm_common::AsmLog;
use strata_checkpoint_types::{BatchInfo, Checkpoint};
use strata_checkpoint_types_ssz::CheckpointTip;
use strata_codec::Codec;
use strata_codec_utils::CodecSsz;
use strata_msg_fmt::TypeId;
use strata_primitives::{epoch::EpochCommitment, l1::BitcoinTxid};

use crate::constants::{CHECKPOINT_TIP_UPDATE_LOG_TYPE, CHECKPOINT_UPDATE_LOG_TYPE};

/// V0 checkpoint log. Emitted by the v0 checkpoint subprotocol.
///
/// Contains full checkpoint metadata including batch info, chainstate transition,
/// and the L1 transaction ID. Superseded by [`CheckpointTipUpdate`] in the main
/// (v1) checkpoint subprotocol.
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
    /// Create a new CheckpointUpdate instance.
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

    /// Construct a `CheckpointUpdate` from a verified checkpoint instance.
    pub fn from_checkpoint(checkpoint: &Checkpoint, checkpoint_txid: BitcoinTxid) -> Self {
        let batch_info = checkpoint.batch_info();

        Self::new(
            batch_info.get_epoch_commitment(),
            batch_info.clone(),
            checkpoint_txid,
        )
    }

    pub fn epoch_commitment(&self) -> EpochCommitment {
        *self.epoch_commitment.inner()
    }

    pub fn batch_info(&self) -> &BatchInfo {
        self.batch_info.inner()
    }

    pub fn checkpoint_txid(&self) -> &BitcoinTxid {
        self.checkpoint_txid.inner()
    }
}

impl AsmLog for CheckpointUpdate {
    const TY: TypeId = CHECKPOINT_UPDATE_LOG_TYPE;
}

/// Records a verified [`CheckpointTip`] update from the v1 checkpoint subprotocol.
///
/// Unlike the v0 [`CheckpointUpdate`], this log only carries the tip
/// (epoch, L1 height, L2 commitment). The inner [`CheckpointTip`] is
/// encoded via [`CodecSsz`] per its SSZ schema.
#[derive(Debug, Clone, Codec)]
pub struct CheckpointTipUpdate {
    /// The new verified checkpoint tip.
    tip: CodecSsz<CheckpointTip>,
}

impl CheckpointTipUpdate {
    /// Creates a new [`CheckpointTipUpdate`] from a [`CheckpointTip`].
    pub fn new(tip: CheckpointTip) -> Self {
        Self {
            tip: CodecSsz::new(tip),
        }
    }

    /// Returns a reference to the checkpoint tip.
    pub fn tip(&self) -> &CheckpointTip {
        self.tip.inner()
    }
}

impl AsmLog for CheckpointTipUpdate {
    const TY: TypeId = CHECKPOINT_TIP_UPDATE_LOG_TYPE;
}

#[cfg(test)]
mod tests {
    use strata_checkpoint_types_ssz::CheckpointTip;
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};

    use super::*;

    #[test]
    fn checkpoint_tip_update_roundtrip() {
        let l2_commitment = OLBlockCommitment::new(42, OLBlockId::from(Buf32::from([0xAB; 32])));
        let tip = CheckpointTip::new(7, 100, l2_commitment);
        let update = CheckpointTipUpdate::new(tip);

        let encoded = encode_to_vec(&update).expect("encoding should not fail");
        let decoded: CheckpointTipUpdate =
            decode_buf_exact(&encoded).expect("decoding should not fail");

        assert_eq!(decoded.tip().epoch, 7);
        assert_eq!(decoded.tip().l1_height, 100);
        assert_eq!(decoded.tip().l2_commitment(), update.tip().l2_commitment());
    }

    #[test]
    fn checkpoint_tip_update_type_id() {
        assert_eq!(
            CheckpointTipUpdate::TY,
            CHECKPOINT_TIP_UPDATE_LOG_TYPE,
            "type ID must match the constant"
        );
    }
}
