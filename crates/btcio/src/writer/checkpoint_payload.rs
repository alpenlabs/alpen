use strata_asm_proto_checkpoint_txs::{CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE};
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_csm_types::L1Payload;
use strata_identifiers::Epoch;

/// Returns the checkpoint epoch encoded in a checkpoint-tagged [`L1Payload`].
///
/// Returns [`None`] for non-checkpoint payloads, multi-chunk payloads, or
/// malformed checkpoint data.
pub fn checkpoint_payload_epoch(payload: &L1Payload) -> Option<Epoch> {
    let tag = payload.tag();
    if tag.subproto_id() != CHECKPOINT_SUBPROTOCOL_ID || tag.tx_type() != OL_STF_CHECKPOINT_TX_TYPE
    {
        return None;
    }

    let [encoded] = payload.data() else {
        return None;
    };
    let decoded: CodecSsz<CheckpointPayload> = decode_buf_exact(encoded).ok()?;
    Some(decoded.into_inner().new_tip().epoch)
}

#[cfg(test)]
mod tests {
    use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
    use strata_codec::encode_to_vec;
    use strata_codec_utils::CodecSsz;
    use strata_csm_types::L1Payload;
    use strata_l1_txfmt::TagData;
    use strata_test_utils_checkpoint::CheckpointTestHarness;

    use super::*;

    #[test]
    fn decodes_checkpoint_epoch() {
        let checkpoint = CheckpointTestHarness::new_random().build_payload();
        let epoch = checkpoint.new_tip().epoch;
        let encoded = encode_to_vec(&CodecSsz::new(checkpoint)).expect("encode checkpoint");
        let payload = L1Payload::new(vec![encoded], OL_STF_CHECKPOINT_TX_TAG.clone())
            .expect("build checkpoint payload");

        assert_eq!(checkpoint_payload_epoch(&payload), Some(epoch));
    }

    #[test]
    fn rejects_non_checkpoint_tag() {
        let payload = L1Payload::new(
            vec![vec![1, 2, 3]],
            TagData::new(1, 1, vec![]).expect("build test tag"),
        )
        .expect("build payload");

        assert_eq!(checkpoint_payload_epoch(&payload), None);
    }

    #[test]
    fn rejects_malformed_checkpoint_data() {
        let payload = L1Payload::new(vec![vec![1, 2, 3]], OL_STF_CHECKPOINT_TX_TAG.clone())
            .expect("build payload");

        assert_eq!(checkpoint_payload_epoch(&payload), None);
    }

    #[test]
    fn rejects_multi_chunk_checkpoint_data() {
        let payload = L1Payload::new(vec![vec![1], vec![2]], OL_STF_CHECKPOINT_TX_TAG.clone())
            .expect("build payload");

        assert_eq!(checkpoint_payload_epoch(&payload), None);
    }
}
