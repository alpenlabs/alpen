//! Extracts DA reveals from Bitcoin blocks via the `OP_RETURN` linking tag.

use alpen_ee_common::{parse_chunk_header, ChunkHeaderParseError, DaChunkHeader};
use bitcoin::{opcodes::all::OP_RETURN, script::Instruction, Block, Script, Transaction, Wtxid};
use strata_l1_envelope_fmt::{errors::EnvelopeParseError, parser::parse_envelope_payload};
use strata_l1_txfmt::MagicBytes;
use thiserror::Error;

/// Parsed reveal data extracted from a single transaction.
#[derive(Debug, Clone)]
pub(crate) struct RevealRecord {
    pub(crate) wtxid: Wtxid,
    pub(crate) prev_wtxid: [u8; 32],
    pub(crate) chunk_header: DaChunkHeader,
    pub(crate) chunk_bytes: Vec<u8>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Used only by scan's own order-preservation test.")
    )]
    pub(crate) block_tx_index: usize,
}

/// Reveal scanning errors.
#[derive(Debug, Error)]
pub(crate) enum ScanError {
    #[error("tag-matched reveal {wtxid} has multiple linking-tag outputs")]
    AmbiguousLinkingTag { wtxid: Wtxid },

    #[error("tag-matched reveal {wtxid} is missing taproot leaf script")]
    MissingLeafScript { wtxid: Wtxid },

    #[error("tag-matched reveal {wtxid} has invalid envelope payload: {source}")]
    InvalidEnvelopePayload {
        wtxid: Wtxid,
        #[source]
        source: EnvelopeParseError,
    },

    #[error("tag-matched reveal {wtxid} has invalid chunk header: {source}")]
    InvalidChunkHeader {
        wtxid: Wtxid,
        #[source]
        source: ChunkHeaderParseError,
    },
}

/// Scans a block and extracts EE-DA reveal records.
pub(crate) fn scan_block(
    block: &Block,
    magic_bytes: MagicBytes,
) -> Result<Vec<RevealRecord>, ScanError> {
    let raw_magic = magic_bytes.into_inner();
    let mut reveals = Vec::new();

    for (block_tx_index, tx) in block.txdata.iter().enumerate() {
        let Some(prev_wtxid) = extract_prev_wtxid_from_linking_tag(tx, raw_magic)? else {
            continue;
        };

        let wtxid = tx.compute_wtxid();
        let leaf_script = tx
            .input
            .first()
            .and_then(|input| input.witness.taproot_leaf_script())
            .ok_or(ScanError::MissingLeafScript { wtxid })?
            .script
            .to_owned();

        let payload = parse_envelope_payload(&leaf_script)
            .map_err(|source| ScanError::InvalidEnvelopePayload { wtxid, source })?;

        let chunk_header = parse_chunk_header(&payload)
            .map_err(|source| ScanError::InvalidChunkHeader { wtxid, source })?;

        reveals.push(RevealRecord {
            wtxid,
            prev_wtxid,
            chunk_header,
            chunk_bytes: payload,
            block_tx_index,
        });
    }

    Ok(reveals)
}

fn extract_prev_wtxid_from_linking_tag(
    tx: &Transaction,
    magic_bytes: [u8; 4],
) -> Result<Option<[u8; 32]>, ScanError> {
    let mut matches = tx
        .output
        .iter()
        .filter_map(|output| parse_linking_tag(output.script_pubkey.as_script(), magic_bytes));

    let Some(prev_wtxid) = matches.next() else {
        return Ok(None);
    };

    if matches.next().is_some() {
        return Err(ScanError::AmbiguousLinkingTag {
            wtxid: tx.compute_wtxid(),
        });
    }

    Ok(Some(prev_wtxid))
}

fn parse_linking_tag(script: &Script, magic_bytes: [u8; 4]) -> Option<[u8; 32]> {
    let mut instructions = script.instructions_minimal();

    match instructions.next()? {
        Ok(Instruction::Op(OP_RETURN)) => {}
        _ => return None,
    }

    match instructions.next()? {
        Ok(Instruction::PushBytes(bytes)) if bytes.as_bytes() == magic_bytes.as_slice() => {}
        _ => return None,
    }

    let prev_wtxid = match instructions.next()? {
        Ok(Instruction::PushBytes(bytes)) if bytes.len() == 32 => {
            let mut prev = [0u8; 32];
            prev.copy_from_slice(bytes.as_bytes());
            prev
        }
        _ => return None,
    };

    if instructions.next().is_some() {
        return None;
    }

    Some(prev_wtxid)
}

#[cfg(test)]
mod tests {
    use bitcoin::script::{Builder, PushBytesBuf};
    use proptest::prelude::*;

    use super::*;
    use crate::{
        l1::test_utils::{
            append_linking_tag_output, build_block_with_txs, build_linking_tag_script,
            build_reveal_tx, magic_bytes_strategy, prev_wtxid_strategy,
        },
        test_utils::{build_chunk_payload, chunk_body_strategy, valid_chunk_header_strategy},
    };

    fn test_magic() -> MagicBytes {
        "ALPN".parse().expect("valid ASCII magic")
    }

    fn test_magic_bytes() -> [u8; 4] {
        test_magic().into_inner()
    }

    proptest! {
        #[test]
        fn parse_linking_tag_accepts_exact_shape(prev in prev_wtxid_strategy()) {
            let script = build_linking_tag_script(test_magic_bytes(), prev);
            prop_assert_eq!(parse_linking_tag(script.as_script(), test_magic_bytes()), Some(prev));
        }

        #[test]
        fn parse_linking_tag_rejects_trailing_pushdata(
            prev in prev_wtxid_strategy(),
            trailing in chunk_body_strategy(16).prop_filter("must be non-empty", |bytes| !bytes.is_empty()),
        ) {
            let trailing = PushBytesBuf::try_from(trailing).expect("length is bounded to <= 16");
            let script = Builder::new()
                .push_opcode(OP_RETURN)
                .push_slice(test_magic_bytes())
                .push_slice(prev)
                .push_slice(trailing)
                .into_script();
            prop_assert_eq!(parse_linking_tag(script.as_script(), test_magic_bytes()), None);
        }

        #[test]
        fn parse_linking_tag_rejects_wrong_magic(
            prev in prev_wtxid_strategy(),
            wrong_magic in magic_bytes_strategy(),
        ) {
            prop_assume!(wrong_magic != test_magic_bytes());
            let script = build_linking_tag_script(wrong_magic, prev);
            prop_assert_eq!(parse_linking_tag(script.as_script(), test_magic_bytes()), None);
        }

        #[test]
        fn scan_block_ignores_non_matching_tag(
            prev in prev_wtxid_strategy(),
            wrong_magic in magic_bytes_strategy(),
            header in valid_chunk_header_strategy(),
            body in chunk_body_strategy(8),
        ) {
            prop_assume!(wrong_magic != test_magic_bytes());
            let (blob_hash, chunk_index, total_chunks) = header;
            let tx = build_reveal_tx(
                wrong_magic,
                prev,
                &build_chunk_payload(blob_hash, chunk_index, total_chunks, &body),
            );
            let block = build_block_with_txs(vec![tx]);
            let records = scan_block(&block, test_magic()).expect("scan succeeds");
            prop_assert!(records.is_empty());
        }

        #[test]
        fn scan_block_errors_when_leaf_script_missing(
            prev in prev_wtxid_strategy(),
            header in valid_chunk_header_strategy(),
            body in chunk_body_strategy(8),
        ) {
            let (blob_hash, chunk_index, total_chunks) = header;
            let mut tx = build_reveal_tx(
                test_magic_bytes(),
                prev,
                &build_chunk_payload(blob_hash, chunk_index, total_chunks, &body),
            );
            tx.input[0].witness.clear();
            let wtxid = tx.compute_wtxid();
            let block = build_block_with_txs(vec![tx]);

            let err = scan_block(&block, test_magic()).expect_err("missing leaf script must fail");
            let is_missing_leaf_script =
                matches!(err, ScanError::MissingLeafScript { wtxid: got } if got == wtxid);
            prop_assert!(is_missing_leaf_script);
        }

        #[test]
        fn scan_block_errors_when_chunk_header_is_invalid(
            prev in prev_wtxid_strategy(),
            bad_header in chunk_body_strategy(36),
        ) {
            let tx = build_reveal_tx(test_magic_bytes(), prev, &bad_header);
            let block = build_block_with_txs(vec![tx]);

            let err = scan_block(&block, test_magic()).expect_err("invalid header must fail");
            let is_invalid_chunk_header = matches!(err, ScanError::InvalidChunkHeader { .. });
            prop_assert!(is_invalid_chunk_header);
        }

        #[test]
        fn scan_block_errors_on_multi_linking_tag(
            prev0 in prev_wtxid_strategy(),
            prev1 in prev_wtxid_strategy(),
            header in valid_chunk_header_strategy(),
            body in chunk_body_strategy(16),
        ) {
            let (blob_hash, chunk_index, total_chunks) = header;
            let mut tx = build_reveal_tx(
                test_magic_bytes(),
                prev0,
                &build_chunk_payload(blob_hash, chunk_index, total_chunks, &body),
            );
            append_linking_tag_output(&mut tx, test_magic_bytes(), prev1);
            let wtxid = tx.compute_wtxid();
            let block = build_block_with_txs(vec![tx]);

            let err = scan_block(&block, test_magic()).expect_err("ambiguous tags must fail");
            let is_ambiguous = matches!(err, ScanError::AmbiguousLinkingTag { wtxid: got } if got == wtxid);
            prop_assert!(is_ambiguous);
        }

        #[test]
        fn scan_block_roundtrips_valid_chunk_header(
            prev in prev_wtxid_strategy(),
            header in valid_chunk_header_strategy(),
            body in chunk_body_strategy(32),
        ) {
            let (expected_blob_hash, chunk_index, total_chunks) = header;
            let expected_payload =
                build_chunk_payload(expected_blob_hash, chunk_index, total_chunks, &body);
            let tx = build_reveal_tx(test_magic_bytes(), prev, &expected_payload);
            let expected_wtxid = tx.compute_wtxid();
            let block = build_block_with_txs(vec![tx]);
            let records = scan_block(&block, test_magic()).expect("scan succeeds");

            prop_assert_eq!(records.len(), 1);
            prop_assert_eq!(records[0].wtxid, expected_wtxid);
            prop_assert_eq!(records[0].prev_wtxid, prev);
            prop_assert_eq!(&records[0].chunk_bytes, &expected_payload);
            let parsed_blob_hash = records[0].chunk_header.blob_hash();
            prop_assert_eq!(
                parsed_blob_hash.as_ref(),
                expected_blob_hash.as_slice()
            );
            prop_assert_eq!(records[0].chunk_header.chunk_index(), chunk_index);
            prop_assert_eq!(records[0].chunk_header.total_chunks(), total_chunks);
        }

        #[test]
        fn build_chunk_payload_matches_parse_chunk_header(
            header in valid_chunk_header_strategy(),
            body in chunk_body_strategy(32),
        ) {
            let (blob_hash, chunk_index, total_chunks) = header;
            let payload = build_chunk_payload(blob_hash, chunk_index, total_chunks, &body);
            let parsed = parse_chunk_header(&payload).expect("round-trip parse must succeed");
            let parsed_blob_hash = parsed.blob_hash();
            prop_assert_eq!(parsed_blob_hash.as_ref(), blob_hash.as_slice());
            prop_assert_eq!(parsed.chunk_index(), chunk_index);
            prop_assert_eq!(parsed.total_chunks(), total_chunks);
        }

        #[test]
        fn scan_block_preserves_block_tx_order(
            prev0 in prev_wtxid_strategy(),
            prev1 in prev_wtxid_strategy(),
            header0 in valid_chunk_header_strategy(),
            header1 in valid_chunk_header_strategy(),
            body0 in chunk_body_strategy(8),
            body1 in chunk_body_strategy(8),
        ) {
            prop_assume!(prev0 != prev1);
            let (blob_hash0, chunk_index0, total_chunks0) = header0;
            let (blob_hash1, chunk_index1, total_chunks1) = header1;
            let tx0 = build_reveal_tx(
                test_magic_bytes(),
                prev0,
                &build_chunk_payload(blob_hash0, chunk_index0, total_chunks0, &body0),
            );
            let tx1 = build_reveal_tx(
                test_magic_bytes(),
                prev1,
                &build_chunk_payload(blob_hash1, chunk_index1, total_chunks1, &body1),
            );
            let block = build_block_with_txs(vec![tx0, tx1]);

            let records = scan_block(&block, test_magic()).expect("scan succeeds");
            prop_assert_eq!(records.len(), 2);
            prop_assert_eq!(records[0].block_tx_index, 0);
            prop_assert_eq!(records[1].block_tx_index, 1);
        }
    }
}
