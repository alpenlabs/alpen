//! Checkpoint publication decisions derived from existing payload and ASM state.

use anyhow::{Context, Result};
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_csm_types::L1Payload;
use strata_identifiers::Epoch;

/// Returns whether a writer payload is a checkpoint already accepted by ASM.
pub(crate) fn is_accepted_checkpoint_payload(
    payload: &L1Payload,
    verified_epoch: Epoch,
) -> Result<bool> {
    let checkpoint_tag = OL_STF_CHECKPOINT_TX_TAG.as_ref();
    if payload.tag().subproto_id() != checkpoint_tag.subproto_id()
        || payload.tag().tx_type() != checkpoint_tag.tx_type()
        || payload.tag().aux_data() != checkpoint_tag.aux_data()
    {
        return Ok(false);
    }

    let encoded = payload.data().concat();
    let checkpoint: CodecSsz<CheckpointPayload> =
        decode_buf_exact(&encoded).context("decode checkpoint writer payload")?;

    Ok(checkpoint.into_inner().new_tip().epoch <= verified_epoch)
}

#[cfg(test)]
mod tests {
    use strata_asm_checkpoint_types::test_utils::create_test_checkpoint_payload;
    use strata_codec::encode_to_vec;
    use strata_l1_txfmt::TagData;

    use super::*;

    fn checkpoint_writer_payload(epoch: u32) -> L1Payload {
        let encoded = encode_to_vec(&CodecSsz::new(create_test_checkpoint_payload(epoch)))
            .expect("encode checkpoint");
        L1Payload::new(vec![encoded], OL_STF_CHECKPOINT_TX_TAG.clone())
            .expect("checkpoint payload fits writer limit")
    }

    #[test]
    fn skips_checkpoint_at_or_below_verified_tip() {
        assert!(
            is_accepted_checkpoint_payload(&checkpoint_writer_payload(14), Epoch::from(14u32))
                .expect("decode checkpoint")
        );
        assert!(
            is_accepted_checkpoint_payload(&checkpoint_writer_payload(14), Epoch::from(28u32))
                .expect("decode checkpoint")
        );
    }

    #[test]
    fn keeps_checkpoint_ahead_of_verified_tip() {
        assert!(
            !is_accepted_checkpoint_payload(&checkpoint_writer_payload(29), Epoch::from(28u32))
                .expect("decode checkpoint")
        );
    }

    #[test]
    fn keeps_non_checkpoint_payload() {
        let payload = L1Payload::new(
            vec![vec![1, 2, 3]],
            TagData::new(0, 0, vec![]).expect("valid tag"),
        )
        .expect("payload fits writer limit");

        assert!(
            !is_accepted_checkpoint_payload(&payload, Epoch::from(28u32))
                .expect("non-checkpoint payload is not decoded")
        );
    }
}
