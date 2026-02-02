//! DA encoding primitives for chunked envelope bundles.
//!
//! Types and functions for splitting, reassembling, and framing DA blobs
//! into Bitcoin envelope chunks for inscription.

use strata_codec::{BufDecoder, Codec, CodecError};
use strata_crypto::hash;
use strata_identifiers::Buf32;

// Bitcoin standardness limit is 400,000 weight units per tx. Reveal tx
// overhead (weight units):
//
// - Non-witness (x4 multiplier):
//   - Version, locktime, counts, outpoint, sequence: ~40 bytes -> 160 wu
//   - OP_RETURN output (36-byte tag + script): ~45 bytes -> 180 wu
//   - Dust output to sequencer: ~35 bytes -> 140 wu
// - Witness (x1 multiplier):
//   - Schnorr signature: 65 wu
//   - Control block: 33 wu
//   - Script overhead (opcodes, pushdata headers): ~50 wu
//   - Chunk header: 37 wu
//
// Total overhead: ~665 wu. Remaining: ~399,335 wu for payload.
// Using 390,000 to keep a safe margin.

/// Maximum chunk payload size in bytes.
const MAX_CHUNK_PAYLOAD: usize = 390_000;

/// Serialized size of [`DaChunkHeader`] in bytes.
const DA_CHUNK_HEADER_SIZE: usize = 37;

/// Current DA chunk encoding version.
///
/// Governs the chunk header layout, payload framing, and reassembly
/// semantics. Bumping this value allows the protocol to evolve the
/// on-chain DA format while remaining backward-compatible.
const DA_CHUNK_ENCODING_VERSION: u8 = 0;

/// SHA-256 hash of the complete, unsplit DA blob.
///
/// Ties all chunks of a blob together for integrity verification during
/// reassembly.
type BlobHash = Buf32;

/// Per-chunk witness header (37 bytes).
///
/// Serialized into the envelope witness alongside the chunk payload.
///
/// ```text
/// offset  size  field
/// 0       1     version
/// 1       32    blob_hash
/// 33      2     chunk_index
/// 35      2     total_chunks
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Codec)]
struct DaChunkHeader {
    version: u8,
    blob_hash: BlobHash,
    chunk_index: u16,
    total_chunks: u16,
}

impl DaChunkHeader {
    /// Validates invariants and constructs a chunk header.
    ///
    /// Returns `None` if `total_chunks` is zero or `chunk_index >= total_chunks`.
    fn new(blob_hash: BlobHash, chunk_index: u16, total_chunks: u16) -> Option<Self> {
        if total_chunks == 0 || chunk_index >= total_chunks {
            return None;
        }
        Some(Self {
            version: DA_CHUNK_ENCODING_VERSION,
            blob_hash,
            chunk_index,
            total_chunks,
        })
    }
}

/// Computes the blob hash (SHA-256) used to tie all chunks together.
fn blob_hash(blob: &[u8]) -> BlobHash {
    hash::raw(blob)
}

/// Splits a blob into chunk payloads.
///
/// Each element is at most [`MAX_CHUNK_PAYLOAD`] bytes. The original blob can
/// be recovered by concatenating all payloads in order.
///
/// # Panics
///
/// Panics if `blob` is empty.
fn split_blob(blob: &[u8]) -> Vec<Vec<u8>> {
    assert!(!blob.is_empty(), "cannot split an empty blob");
    blob.chunks(MAX_CHUNK_PAYLOAD)
        .map(|c| c.to_vec())
        .collect()
}

/// Reassembles a blob from ordered chunk payloads and verifies integrity.
///
/// Chunks must already be sorted by index. Verifies the concatenated result
/// against `expected_hash`. Returns `None` on empty input or hash mismatch.
fn reassemble_from_payloads(
    payloads: &[Vec<u8>],
    expected_hash: BlobHash,
) -> Option<Vec<u8>> {
    if payloads.is_empty() {
        return None;
    }
    let blob: Vec<u8> = payloads.iter().flat_map(|c| c.iter().copied()).collect();
    if blob_hash(&blob) != expected_hash {
        return None;
    }
    Some(blob)
}

/// Encodes a single DA chunk: header ++ payload.
///
/// The returned bytes go inside the envelope witness (after the tag bytes,
/// which are added by the envelope builder).
fn encode_da_chunk(header: &DaChunkHeader, payload: &[u8]) -> Result<Vec<u8>, CodecError> {
    let mut buf = strata_codec::encode_to_vec(header)?;
    buf.extend_from_slice(payload);
    Ok(buf)
}

/// Decodes a DA chunk from envelope witness data into header + payload.
fn decode_da_chunk(data: &[u8]) -> Result<(DaChunkHeader, &[u8]), CodecError> {
    if data.len() < DA_CHUNK_HEADER_SIZE {
        return Err(CodecError::MalformedField("data shorter than chunk header"));
    }
    let mut dec = BufDecoder::new(&data[..DA_CHUNK_HEADER_SIZE]);
    let header = DaChunkHeader::decode(&mut dec)?;
    Ok((header, &data[DA_CHUNK_HEADER_SIZE..]))
}

/// Splits a blob into encoded DA chunks ready for envelope inscription.
///
/// Each returned `Vec<u8>` contains a serialized [`DaChunkHeader`] followed
/// by the chunk payload — the format expected by [`decode_da_chunk`].
///
/// # Panics
///
/// Panics if `blob` is empty.
pub fn prepare_da_chunks(blob: &[u8]) -> Result<Vec<Vec<u8>>, CodecError> {
    let hash = blob_hash(blob);
    let payloads = split_blob(blob);
    let total_chunks = u16::try_from(payloads.len()).map_err(|_| {
        CodecError::MalformedField("blob too large: chunk count exceeds u16::MAX")
    })?;

    payloads
        .iter()
        .enumerate()
        .map(|(i, payload)| {
            let header = DaChunkHeader::new(hash, i as u16, total_chunks)
                .expect("index < total_chunks by construction");
            encode_da_chunk(&header, payload)
        })
        .collect()
}

/// Reassembles a blob from raw encoded chunks (header ++ payload each).
///
/// Performs the full pipeline: decode headers, validate consistency,
/// order by `chunk_index`, concatenate payloads, and verify SHA-256 hash.
/// Returns `None` on any validation failure.
pub fn reassemble_from_da_chunks(encoded_chunks: &[Vec<u8>]) -> Option<Vec<u8>> {
    if encoded_chunks.is_empty() {
        return None;
    }

    let mut decoded: Vec<(DaChunkHeader, &[u8])> = Vec::with_capacity(encoded_chunks.len());
    for enc in encoded_chunks {
        decoded.push(decode_da_chunk(enc).ok()?);
    }

    let expected_hash = decoded[0].0.blob_hash;
    let total_chunks = decoded[0].0.total_chunks;

    if total_chunks as usize != decoded.len() {
        return None;
    }

    for (header, _) in &decoded[1..] {
        if header.blob_hash != expected_hash || header.total_chunks != total_chunks {
            return None;
        }
    }

    decoded.sort_by_key(|(h, _)| h.chunk_index);

    for (i, (header, _)) in decoded.iter().enumerate() {
        if header.chunk_index != i as u16 {
            return None;
        }
    }

    let payloads: Vec<Vec<u8>> = decoded.iter().map(|(_, p)| p.to_vec()).collect();
    reassemble_from_payloads(&payloads, expected_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_blob(size: usize) -> Vec<u8> {
        (0..size).map(|i| (i % 256) as u8).collect()
    }

    #[test]
    fn chunk_header_codec_produces_exact_size() {
        let header = DaChunkHeader::new(Buf32::from([0x42; 32]), 3, 10).unwrap();
        let encoded = strata_codec::encode_to_vec(&header).unwrap();
        assert_eq!(encoded.len(), DA_CHUNK_HEADER_SIZE);
        let decoded: DaChunkHeader = strata_codec::decode_buf_exact(&encoded).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn split_and_reassemble_roundtrip() {
        for size in [1, 100, MAX_CHUNK_PAYLOAD, MAX_CHUNK_PAYLOAD * 2 + 100] {
            let blob = make_test_blob(size);
            let hash = blob_hash(&blob);
            let chunks = split_blob(&blob);
            let result = reassemble_from_payloads(&chunks, hash).unwrap();
            assert_eq!(result, blob);
        }
    }

    #[test]
    fn full_pipeline_roundtrip() {
        let blob = make_test_blob(MAX_CHUNK_PAYLOAD + 1234);
        let encoded = prepare_da_chunks(&blob).unwrap();
        assert_eq!(reassemble_from_da_chunks(&encoded).unwrap(), blob);
    }

    #[test]
    fn full_pipeline_handles_unordered_input() {
        let blob = make_test_blob(MAX_CHUNK_PAYLOAD * 2 + 100);
        let mut encoded = prepare_da_chunks(&blob).unwrap();
        encoded.reverse();
        assert_eq!(reassemble_from_da_chunks(&encoded).unwrap(), blob);
    }

    #[test]
    fn full_pipeline_rejects_invalid_input() {
        assert!(reassemble_from_da_chunks(&[]).is_none());
        assert!(reassemble_from_da_chunks(&[vec![0xFF; 10]]).is_none());

        let blob = make_test_blob(MAX_CHUNK_PAYLOAD + 100);
        let mut encoded = prepare_da_chunks(&blob).unwrap();
        encoded.remove(1);
        assert!(reassemble_from_da_chunks(&encoded).is_none());

        let blob2 = make_test_blob(MAX_CHUNK_PAYLOAD + 100);
        let payloads = split_blob(&blob2);
        let hash = blob_hash(&blob2);
        let total = payloads.len() as u16;
        let tampered = vec![
            encode_da_chunk(&DaChunkHeader::new(hash, 0, total).unwrap(), &payloads[0]).unwrap(),
            encode_da_chunk(
                &DaChunkHeader::new(Buf32::from([0xFF; 32]), 1, total).unwrap(),
                &payloads[1],
            )
            .unwrap(),
        ];
        assert!(reassemble_from_da_chunks(&tampered).is_none());
    }
}
