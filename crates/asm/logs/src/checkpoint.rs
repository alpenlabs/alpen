use bitcoin::{Txid, hashes::Hash};
use ssz::{Decode, Encode};
use strata_asm_common::AsmLog;
use strata_btc_types::BitcoinTxid;
use strata_checkpoint_types::{BatchInfo, Checkpoint};
use strata_checkpoint_types_ssz::CheckpointTip;
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};
use strata_codec_utils::CodecSsz;
use strata_identifiers::EpochCommitment;
use strata_msg_fmt::TypeId;

use crate::constants::{CHECKPOINT_TIP_UPDATE_LOG_TYPE, CHECKPOINT_UPDATE_LOG_TYPE};
pub use crate::ssz_generated::ssz::checkpoint::{
    BatchInfoBytes, CheckpointUpdate, CheckpointUpdateRef, EpochCommitmentBytes,
};

fn encode_epoch_commitment(epoch_commitment: EpochCommitment) -> EpochCommitmentBytes {
    EpochCommitmentBytes::new(epoch_commitment.as_ssz_bytes())
        .expect("epoch commitment must stay within SSZ bounds")
}

fn decode_epoch_commitment(bytes: &[u8]) -> EpochCommitment {
    EpochCommitment::from_ssz_bytes(bytes).expect("epoch commitment bytes must remain valid")
}

/// V0 checkpoint log. Emitted by the v0 checkpoint subprotocol.
///
/// Contains full checkpoint metadata including batch info, chainstate transition,
/// and the L1 transaction ID. Superseded by [`CheckpointTipUpdate`] in the main
/// (v1) checkpoint subprotocol.
impl CheckpointUpdate {
    /// Create a new CheckpointUpdate instance.
    pub fn new(
        epoch_commitment: EpochCommitment,
        batch_info: BatchInfo,
        checkpoint_txid: BitcoinTxid,
    ) -> Self {
        Self {
            epoch_commitment: encode_epoch_commitment(epoch_commitment),
            batch_info: batch_info.to_legacy_bytes().into(),
            checkpoint_txid: checkpoint_txid.inner_raw().into(),
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
        decode_epoch_commitment(&self.epoch_commitment)
    }

    pub fn batch_info(&self) -> BatchInfo {
        BatchInfo::from_legacy_bytes(&self.batch_info)
            .expect("checkpoint update batch info is valid")
    }

    pub fn checkpoint_txid(&self) -> BitcoinTxid {
        let txid_bytes: [u8; 32] = self
            .checkpoint_txid
            .as_ref()
            .try_into()
            .expect("checkpoint txid must remain 32 bytes");
        let txid = Txid::from_byte_array(txid_bytes);
        BitcoinTxid::new(&txid)
    }
}

impl Codec for CheckpointUpdate {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = Varint::decode(dec)?;
        let len_usize = len.inner() as usize;
        let mut buffer = vec![0u8; len_usize];
        dec.read_buf(&mut buffer)?;
        Self::from_ssz_bytes(&buffer).map_err(|_| CodecError::MalformedField("ssz"))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let bytes = self.as_ssz_bytes();
        let len = Varint::new_usize(bytes.len()).ok_or(CodecError::OverflowContainer)?;
        len.encode(enc)?;
        enc.write_buf(&bytes)
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
