use strata_asm_common::AsmLog;
use strata_checkpoint_types::{BatchInfo, Checkpoint};
use strata_checkpoint_types_ssz::CheckpointTip;
use strata_codec::Codec;
use strata_codec_utils::CodecSsz;
use strata_identifiers::Buf32;
use strata_msg_fmt::TypeId;
use strata_primitives::{epoch::EpochCommitment, l1::BitcoinTxid};

use crate::constants::{CHECKPOINT_TIP_UPDATE_LOG_TYPE, CHECKPOINT_UPDATE_LOG_TYPE};

/// V0 checkpoint log emitted by the v0 checkpoint subprotocol.
///
/// Contains full checkpoint metadata: epoch commitment, batch info, and the L1
/// transaction ID that carried the checkpoint proof.
///
/// Superseded in the main (v1) checkpoint subprotocol by [`CheckpointTipUpdate`].
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

/// V1 checkpoint tip update log emitted by the main checkpoint subprotocol.
///
/// Records a verified [`CheckpointTip`] (epoch, L1 height, L2 commitment)
/// together with the L1 transaction ID that carried the checkpoint proof.
///
/// The inner [`CheckpointTip`] is encoded via [`CodecSsz`] according to its SSZ schema.
#[derive(Debug, Clone, Codec)]
pub struct CheckpointTipUpdate {
    /// The new verified checkpoint tip.
    tip: CodecSsz<CheckpointTip>,

    /// Txid of the L1 transaction that carried the checkpoint tx.
    checkpoint_txid: Buf32,
}

impl CheckpointTipUpdate {
    /// Creates a new [`CheckpointTipUpdate`] from a [`CheckpointTip`] and the
    /// raw txid bytes of the L1 transaction that carried the checkpoint.
    pub fn new(tip: CheckpointTip, checkpoint_txid: Buf32) -> Self {
        Self {
            tip: CodecSsz::new(tip),
            checkpoint_txid,
        }
    }

    /// Returns a reference to the checkpoint tip.
    pub fn tip(&self) -> &CheckpointTip {
        self.tip.inner()
    }

    /// Returns the checkpoint L1 transaction ID as raw bytes.
    pub fn checkpoint_txid(&self) -> &Buf32 {
        &self.checkpoint_txid
    }
}

impl AsmLog for CheckpointTipUpdate {
    const TY: TypeId = CHECKPOINT_TIP_UPDATE_LOG_TYPE;
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_checkpoint_types_ssz::{test_utils::checkpoint_tip_strategy, CheckpointTip};
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_identifiers::{test_utils::buf32_strategy, Buf32, OLBlockCommitment, OLBlockId};
    use strata_test_utils::ArbitraryGenerator;

    use super::*;

    #[test]
    fn checkpoint_tip_update_roundtrip() {
        let mut arb = ArbitraryGenerator::new();
        let l2_commitment = OLBlockCommitment::new(42, OLBlockId::from(arb.generate::<Buf32>()));
        let tip = CheckpointTip::new(7, 100, l2_commitment);
        let txid: Buf32 = arb.generate();
        let update = CheckpointTipUpdate::new(tip, txid);

        let encoded = encode_to_vec(&update).expect("encoding should not fail");
        let decoded: CheckpointTipUpdate =
            decode_buf_exact(&encoded).expect("decoding should not fail");

        assert_eq!(decoded.tip().epoch, 7);
        assert_eq!(decoded.tip().l1_height, 100);
        assert_eq!(decoded.tip().l2_commitment(), update.tip().l2_commitment());
        assert_eq!(decoded.checkpoint_txid(), update.checkpoint_txid());
    }

    proptest! {
        #[test]
        fn checkpoint_tip_update_roundtrip_proptest(
            tip in checkpoint_tip_strategy(),
            txid_bytes in buf32_strategy(),
        ) {
            let update = CheckpointTipUpdate::new(tip, txid_bytes);

            let encoded = encode_to_vec(&update).expect("encoding should not fail");
            let decoded: CheckpointTipUpdate =
                decode_buf_exact(&encoded).expect("decoding should not fail");

            prop_assert_eq!(decoded.tip().epoch, update.tip().epoch);
            prop_assert_eq!(decoded.tip().l1_height, update.tip().l1_height);
            prop_assert_eq!(decoded.tip().l2_commitment(), update.tip().l2_commitment());
            prop_assert_eq!(decoded.checkpoint_txid(), update.checkpoint_txid());
        }
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
