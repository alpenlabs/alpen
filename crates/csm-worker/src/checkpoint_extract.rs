//! Locate the L1 transaction carrying a validated checkpoint.

use bitcoin::{Block, hashes::Hash};
use strata_asm_common::TxInputRef;
use strata_asm_proto_checkpoint_txs::{
    CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE, extract_checkpoint_from_envelope,
};
use strata_asm_proto_checkpoint_types::{CheckpointPayload, CheckpointTip};
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use strata_primitives::buf::Buf32;
use tracing::{debug, error, warn};

/// Identification of the L1 transaction that carries a validated checkpoint,
/// along with its decoded payload.
#[derive(Debug, Clone)]
pub(crate) struct ExtractedCheckpoint {
    pub(crate) txid: Buf32,
    pub(crate) wtxid: Buf32,
    pub(crate) payload: CheckpointPayload,
}

/// Returns the SPS-50-tagged checkpoint envelope tx in `block` whose payload
/// matches `expected` on `(epoch, l2_commitment)`, or `None` if none does.
/// Keeps the first match and logs an error if multiple match.
pub(crate) fn extract_matching_checkpoint(
    block: &Block,
    magic: MagicBytes,
    expected: &CheckpointTip,
) -> Option<ExtractedCheckpoint> {
    let parser = ParseConfig::new(magic);
    let mut found: Option<ExtractedCheckpoint> = None;

    for tx in &block.txdata {
        let Ok(tag) = parser.try_parse_tx(tx) else {
            continue;
        };
        if tag.subproto_id() != CHECKPOINT_SUBPROTOCOL_ID
            || tag.tx_type() != OL_STF_CHECKPOINT_TX_TYPE
        {
            continue;
        }

        let tx_input = TxInputRef::new(tx, tag);
        let envelope = match extract_checkpoint_from_envelope(&tx_input) {
            Ok(env) => env,
            Err(e) => {
                warn!(
                    txid = ?tx.compute_txid(),
                    error = ?e,
                    "failed to parse checkpoint envelope; skipping"
                );
                continue;
            }
        };

        let new_tip = envelope.payload.new_tip();
        if new_tip.epoch != expected.epoch || new_tip.l2_commitment() != expected.l2_commitment() {
            continue;
        }

        let txid = Buf32::from(tx.compute_txid().to_byte_array());
        let wtxid = Buf32::from(tx.compute_wtxid().to_byte_array());
        let candidate = ExtractedCheckpoint {
            txid,
            wtxid,
            payload: envelope.payload,
        };

        if found.is_some() {
            error!(
                ?txid,
                epoch = expected.epoch,
                "multiple checkpoint txs match the validated tip; keeping the first"
            );
            continue;
        }
        debug!(
            ?txid,
            ?wtxid,
            epoch = expected.epoch,
            "matched checkpoint tx"
        );
        found = Some(candidate);
    }

    found
}

#[cfg(test)]
mod tests {
    use bitcoin::{Block, Transaction, absolute::LockTime};
    use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
    use strata_asm_proto_checkpoint_types::{
        CheckpointPayload, test_utils::create_test_checkpoint_payload,
    };
    use strata_asm_proto_txs_test_utils::{
        TEST_MAGIC_BYTES, create_dummy_tx, create_reveal_transaction_stub,
    };
    use strata_codec::encode_to_vec;
    use strata_codec_utils::CodecSsz;
    use strata_l1_txfmt::TagData;

    use super::*;

    fn checkpoint_envelope_tx(payload: &CheckpointPayload) -> Transaction {
        let bytes = encode_to_vec(&CodecSsz::new(payload.clone())).expect("encode payload");
        create_reveal_transaction_stub(bytes, &OL_STF_CHECKPOINT_TX_TAG)
    }

    fn create_block_from_txs(txs: Vec<Transaction>) -> Block {
        // Header values don't matter for the extractor; only txdata is read.
        Block {
            header: bitcoin::block::Header {
                version: bitcoin::block::Version::TWO,
                prev_blockhash: bitcoin::BlockHash::from_raw_hash(
                    bitcoin::hashes::sha256d::Hash::all_zeros(),
                ),
                merkle_root: bitcoin::TxMerkleNode::from_raw_hash(
                    bitcoin::hashes::sha256d::Hash::all_zeros(),
                ),
                time: 0,
                bits: bitcoin::CompactTarget::from_consensus(0),
                nonce: 0,
            },
            txdata: txs,
        }
    }

    #[test]
    fn matches_single_checkpoint_tx() {
        let payload = create_test_checkpoint_payload(7);
        let expected = *payload.new_tip();
        let tx = checkpoint_envelope_tx(&payload);
        let block = create_block_from_txs(vec![create_dummy_tx(1, 1), tx.clone()]);

        let result =
            extract_matching_checkpoint(&block, TEST_MAGIC_BYTES, &expected).expect("should match");
        assert_eq!(result.payload.new_tip().epoch, 7);
        assert_eq!(result.txid, Buf32::from(tx.compute_txid().to_byte_array()));
        assert_eq!(
            result.wtxid,
            Buf32::from(tx.compute_wtxid().to_byte_array())
        );
    }

    #[test]
    fn returns_none_when_no_envelope_matches_expected_tip() {
        let other = create_test_checkpoint_payload(2);
        let expected = *create_test_checkpoint_payload(99).new_tip();
        let block = create_block_from_txs(vec![checkpoint_envelope_tx(&other)]);

        assert!(extract_matching_checkpoint(&block, TEST_MAGIC_BYTES, &expected).is_none());
    }

    #[test]
    fn picks_checkpoint_tx_alongside_other_subprotocol_txs() {
        let payload = create_test_checkpoint_payload(5);
        let expected = *payload.new_tip();

        // Same envelope payload but tagged with a foreign subprotocol id.
        let foreign_tag = TagData::new(99, 1, vec![]).expect("tag");
        let bytes = encode_to_vec(&CodecSsz::new(payload.clone())).expect("encode");
        let foreign_tx = create_reveal_transaction_stub(bytes, &foreign_tag);

        let real_tx = checkpoint_envelope_tx(&payload);
        let block = create_block_from_txs(vec![foreign_tx, real_tx.clone()]);

        let result = extract_matching_checkpoint(&block, TEST_MAGIC_BYTES, &expected)
            .expect("should match the real tx");
        assert_eq!(
            result.txid,
            Buf32::from(real_tx.compute_txid().to_byte_array())
        );
    }

    #[test]
    fn keeps_first_match_when_multiple_match() {
        let payload = create_test_checkpoint_payload(11);
        let expected = *payload.new_tip();
        let tx1 = checkpoint_envelope_tx(&payload);
        let mut tx2 = checkpoint_envelope_tx(&payload);
        // Bump lock_time so tx2 has a different txid than tx1.
        tx2.lock_time = LockTime::from_height(1).unwrap();
        assert_ne!(tx1.compute_txid(), tx2.compute_txid());
        let block = create_block_from_txs(vec![tx1.clone(), tx2]);

        let result =
            extract_matching_checkpoint(&block, TEST_MAGIC_BYTES, &expected).expect("should match");
        assert_eq!(result.txid, Buf32::from(tx1.compute_txid().to_byte_array()));
    }
}
