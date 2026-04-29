use alpen_ee_common::parse_chunk_header;
use bitcoin::{hashes::Hash as _, Wtxid};
use proptest::{collection, prelude::*};

use crate::l1::scan::RevealRecord;

/// Builds a v0 DA chunk payload (`header ++ payload`).
///
/// This helper round-trips through [`parse_chunk_header`] so fixtures stay
/// aligned with the production parser contract.
pub(crate) fn build_chunk_payload(
    blob_hash: [u8; 32],
    chunk_index: u16,
    total_chunks: u16,
    body: &[u8],
) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(0);
    payload.extend_from_slice(&blob_hash);
    payload.extend_from_slice(&chunk_index.to_be_bytes());
    payload.extend_from_slice(&total_chunks.to_be_bytes());
    payload.extend_from_slice(body);

    let parsed = parse_chunk_header(&payload).expect("test chunk payload must have a valid header");
    assert_eq!(parsed.chunk_index(), chunk_index);
    assert_eq!(parsed.total_chunks(), total_chunks);
    assert_eq!(parsed.blob_hash().as_ref(), blob_hash.as_slice());

    payload
}

/// Strategy for valid chunk-body bytes with caller-controlled max length.
pub(crate) fn chunk_body_strategy(max_len: usize) -> impl Strategy<Value = Vec<u8>> {
    collection::vec(any::<u8>(), 0..=max_len)
}

/// Strategy for valid chunk-header parameters (`chunk_index < total_chunks`).
pub(crate) fn valid_chunk_header_strategy() -> impl Strategy<Value = ([u8; 32], u16, u16)> {
    (any::<[u8; 32]>(), 1u16..=u16::MAX).prop_flat_map(|(blob_hash, total_chunks)| {
        (Just(blob_hash), 0u16..total_chunks, Just(total_chunks))
    })
}

/// Builds a reveal record with a chunk header parsed from encoded chunk bytes.
pub(crate) fn build_reveal_record(
    wtxid_bytes: [u8; 32],
    prev_wtxid: [u8; 32],
    blob_hash: [u8; 32],
    chunk_index: u16,
    total_chunks: u16,
    body: &[u8],
    block_tx_index: usize,
) -> RevealRecord {
    let chunk_bytes = build_chunk_payload(blob_hash, chunk_index, total_chunks, body);
    let chunk_header = parse_chunk_header(&chunk_bytes).expect("constructed payload is valid");

    RevealRecord {
        wtxid: Wtxid::from_byte_array(wtxid_bytes),
        prev_wtxid,
        chunk_header,
        chunk_bytes,
        block_tx_index,
    }
}
