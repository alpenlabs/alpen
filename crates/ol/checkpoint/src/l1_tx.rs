//! Identifies the checkpoints carried by L1 writer payloads and Bitcoin transactions.
//!
//! Both directions of the same question. The writer queue holds pre-envelope
//! [`L1Payload`]s waiting to be signed; the broadcaster holds the envelope
//! transactions built from them. Startup reconciliation has to line the two up, and
//! the watcher has to recognize a checkpoint before it decides whether publishing one
//! is still worthwhile.

use bitcoin::Transaction;
use ssz::Encode;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::TxInputRef;
use strata_asm_proto_checkpoint_txs::{
    CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE, extract_checkpoint_from_envelope,
};
use strata_btcio::writer::{CheckpointPayloadInspector, PayloadCheckpointRef};
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_crypto::hash;
use strata_csm_types::L1Payload;
use strata_identifiers::{Buf32, Epoch};
use strata_l1_txfmt::{MagicBytes, ParseConfig};

/// A checkpoint recognized on an L1 transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointTxRef {
    /// Epoch the checkpoint attests to.
    pub epoch: Epoch,
    /// Identity of the checkpoint body, from [`checkpoint_payload_id`].
    pub id: Buf32,
}

/// Returns the identity of a checkpoint body.
///
/// This is the hash of the SSZ encoding, which is what the sequencer already uses as
/// the `PayloadIntent` commitment when it queues a checkpoint
/// (`bin/strata/src/sequencer/rpc.rs`). Hashing the SSZ body rather than the framed
/// bytes lets the writer-queue and envelope-transaction paths agree without either
/// having to reproduce the other's `strata-codec` framing.
///
/// Two checkpoints for the same epoch are distinct here whenever anything in the body
/// differs, including the proof. An epoch number alone does not identify a checkpoint:
/// a fork or a rebuild can leave several candidates for one epoch in flight.
pub fn checkpoint_payload_id(payload: &CheckpointPayload) -> Buf32 {
    hash::raw(&payload.as_ssz_bytes())
}

/// Identifies the checkpoint in a checkpoint-tagged [`L1Payload`].
///
/// The writer queue carries payloads for every subprotocol, so the SPS-50 tag is what
/// makes this checkpoint-specific: a payload not tagged with
/// [`CHECKPOINT_SUBPROTOCOL_ID`] and [`OL_STF_CHECKPOINT_TX_TYPE`] belongs to someone
/// else and yields [`PayloadCheckpointRef::NotCheckpoint`].
///
/// These are the pre-envelope bytes stored when a checkpoint intent is submitted (the
/// sequencer's `complete_checkpoint_signature` RPC): one chunk holding a
/// [`CheckpointPayload`] encoded as [`CodecSsz`], which frames the SSZ body behind a
/// `strata-codec` varint length prefix. That prefix is why decoding runs through
/// [`decode_buf_exact`] rather than SSZ directly, and why a multi-chunk payload cannot
/// be one the checkpoint path produced.
///
/// Anything tagged as a checkpoint that will not decode yields
/// [`PayloadCheckpointRef::Undecodable`], leaving the fallback to the caller.
pub fn inspect_l1_payload(payload: &L1Payload) -> PayloadCheckpointRef {
    let tag = payload.tag();
    if tag.subproto_id() != CHECKPOINT_SUBPROTOCOL_ID || tag.tx_type() != OL_STF_CHECKPOINT_TX_TYPE
    {
        return PayloadCheckpointRef::NotCheckpoint;
    }

    let [encoded] = payload.data() else {
        return PayloadCheckpointRef::Undecodable;
    };
    let Ok(decoded) = decode_buf_exact::<CodecSsz<CheckpointPayload>>(encoded) else {
        return PayloadCheckpointRef::Undecodable;
    };

    let checkpoint = decoded.into_inner();
    PayloadCheckpointRef::Checkpoint {
        epoch: checkpoint.new_tip().epoch,
        id: checkpoint_payload_id(&checkpoint),
    }
}

/// Identifies the checkpoint carried by an envelope transaction, if it carries one.
///
/// Yields [`None`] for anything that is not an SPS-50 checkpoint envelope under
/// `magic_bytes`, and for envelopes whose payload will not decode.
pub fn checkpoint_from_tx(tx: &Transaction, magic_bytes: MagicBytes) -> Option<CheckpointTxRef> {
    let tag = ParseConfig::new(magic_bytes).try_parse_tx(tx).ok()?;
    if tag.subproto_id() != CHECKPOINT_SUBPROTOCOL_ID || tag.tx_type() != OL_STF_CHECKPOINT_TX_TYPE
    {
        return None;
    }

    let envelope = extract_checkpoint_from_envelope(&TxInputRef::new(tx, tag)).ok()?;
    Some(CheckpointTxRef {
        epoch: envelope.payload.new_tip().epoch,
        id: checkpoint_payload_id(&envelope.payload),
    })
}

/// [`CheckpointPayloadInspector`] backed by the ASM checkpoint transaction format.
#[derive(Clone, Copy, Debug, Default)]
pub struct AsmCheckpointInspector;

impl CheckpointPayloadInspector for AsmCheckpointInspector {
    fn inspect_payload(&self, payload: &L1Payload) -> PayloadCheckpointRef {
        inspect_l1_payload(payload)
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
    use strata_asm_proto_txs_test_utils::{TEST_MAGIC_BYTES, create_reveal_transaction_stub};
    use strata_codec::encode_to_vec;
    use strata_l1_txfmt::TagData;
    use strata_test_utils_checkpoint::CheckpointTestHarness;

    use super::*;

    fn encode(checkpoint: &CheckpointPayload) -> Vec<u8> {
        encode_to_vec(&CodecSsz::new(checkpoint.clone())).expect("encode checkpoint")
    }

    fn l1_payload(checkpoint: &CheckpointPayload) -> L1Payload {
        L1Payload::new(vec![encode(checkpoint)], OL_STF_CHECKPOINT_TX_TAG.clone())
            .expect("build checkpoint payload")
    }

    #[test]
    fn decodes_checkpoint_epoch() {
        let checkpoint = CheckpointTestHarness::new_random().build_payload();
        let epoch = checkpoint.new_tip().epoch;

        assert_eq!(
            inspect_l1_payload(&l1_payload(&checkpoint)),
            PayloadCheckpointRef::Checkpoint {
                epoch,
                id: checkpoint_payload_id(&checkpoint),
            }
        );
    }

    /// A tag belonging to another subprotocol. Spelled relative to the checkpoint
    /// constants so it stays a non-checkpoint tag if the ids are ever renumbered.
    fn other_subproto_tag() -> TagData {
        TagData::new(
            CHECKPOINT_SUBPROTOCOL_ID + 1,
            OL_STF_CHECKPOINT_TX_TYPE,
            vec![],
        )
        .expect("build test tag")
    }

    #[test]
    fn rejects_other_subprotocol_tag() {
        let checkpoint = CheckpointTestHarness::new_random().build_payload();
        let payload =
            L1Payload::new(vec![encode(&checkpoint)], other_subproto_tag()).expect("build payload");

        assert_eq!(
            inspect_l1_payload(&payload),
            PayloadCheckpointRef::NotCheckpoint
        );
    }

    #[test]
    fn rejects_other_tx_type_tag() {
        let checkpoint = CheckpointTestHarness::new_random().build_payload();
        let tag = TagData::new(
            CHECKPOINT_SUBPROTOCOL_ID,
            OL_STF_CHECKPOINT_TX_TYPE + 1,
            vec![],
        )
        .expect("build test tag");
        let payload = L1Payload::new(vec![encode(&checkpoint)], tag).expect("build payload");

        assert_eq!(
            inspect_l1_payload(&payload),
            PayloadCheckpointRef::NotCheckpoint
        );
    }

    #[test]
    fn rejects_malformed_checkpoint_data() {
        let payload = L1Payload::new(vec![vec![1, 2, 3]], OL_STF_CHECKPOINT_TX_TAG.clone())
            .expect("build payload");

        assert_eq!(
            inspect_l1_payload(&payload),
            PayloadCheckpointRef::Undecodable
        );
    }

    #[test]
    fn rejects_multi_chunk_checkpoint_data() {
        let payload = L1Payload::new(vec![vec![1], vec![2]], OL_STF_CHECKPOINT_TX_TAG.clone())
            .expect("build payload");

        assert_eq!(
            inspect_l1_payload(&payload),
            PayloadCheckpointRef::Undecodable
        );
    }

    /// The writer-queue and envelope-transaction paths must agree on identity, since
    /// startup reconciliation matches a queued bundle against an escaped envelope by
    /// comparing the two.
    #[test]
    fn payload_and_tx_paths_agree_on_identity() {
        let checkpoint = CheckpointTestHarness::new_random().build_payload();
        let tx = create_reveal_transaction_stub(encode(&checkpoint), &OL_STF_CHECKPOINT_TX_TAG);

        let from_tx = checkpoint_from_tx(&tx, TEST_MAGIC_BYTES).expect("recognize checkpoint tx");
        let PayloadCheckpointRef::Checkpoint { epoch, id } =
            inspect_l1_payload(&l1_payload(&checkpoint))
        else {
            panic!("expected a checkpoint payload");
        };

        assert_eq!(from_tx.id, id);
        assert_eq!(from_tx.epoch, epoch);
    }

    /// Two checkpoints for one epoch are distinct identities. This is what keeps
    /// reconciliation from relinking a queued bundle to a different candidate's
    /// in-flight envelope.
    #[test]
    fn same_epoch_candidates_have_distinct_identities() {
        let harness = CheckpointTestHarness::new_random();
        let tip = harness.gen_new_tip();
        let first = harness.build_payload_with_tip(tip);
        let second = harness.build_payload_with_tip(tip);

        assert_eq!(first.new_tip().epoch, second.new_tip().epoch);
        assert_ne!(
            checkpoint_payload_id(&first),
            checkpoint_payload_id(&second)
        );
    }

    #[test]
    fn ignores_non_checkpoint_tx() {
        let checkpoint = CheckpointTestHarness::new_random().build_payload();
        let tx = create_reveal_transaction_stub(encode(&checkpoint), &other_subproto_tag());

        assert_eq!(checkpoint_from_tx(&tx, TEST_MAGIC_BYTES), None);
    }
}
