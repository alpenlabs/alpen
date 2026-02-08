//! DA encoding primitives for chunked envelope bundles.
//!
//! Types and functions for splitting, reassembling, and framing DA blobs
//! into Bitcoin envelope chunks for inscription.

use alpen_reth_statediff::BatchStateDiff;
use strata_codec::{decode_buf_exact, encode_to_vec, BufDecoder, Codec, CodecError};
use strata_crypto::hash;
use strata_identifiers::Buf32;

use crate::BatchId;

/// Compact summary of the last EVM block header in a batch.
///
/// Captures the subset of the EVM block header needed to build the next
/// block during DA-only chain reconstruction. A new sequencer recovering
/// from L1 DA has the [`BatchStateDiff`] (account/storage changes) but
/// **not** the block headers themselves — these fields fill that gap.
///
/// - `base_fee`, `gas_used`, `gas_limit` feed the EIP-1559 base-fee calculation and gas-limit
///   adjustment for the next block.
/// - `timestamp` enforces monotonicity (`next > parent`).
/// - `block_num` identifies where the chain continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Codec)]
pub struct EvmHeaderDigest {
    /// Block number of the last EVM block in this batch.
    pub block_num: u64,
    /// Unix timestamp (seconds) of the last EVM block.
    pub timestamp: u64,
    /// Base fee per gas (EIP-1559) of the last EVM block.
    pub base_fee: u64,
    /// Total gas consumed by the last EVM block.
    pub gas_used: u64,
    /// Gas limit of the last EVM block.
    pub gas_limit: u64,
}

/// DA blob containing batch metadata and state diff.
///
/// This is the top-level structure that gets encoded and posted to L1.
/// It wraps the batch state diff with identification metadata needed for
/// L1 sync and chain reconstruction.
#[derive(Debug, Clone, Codec)]
pub struct DaBlob {
    /// Batch identifier (prev_block_hash, last_block_hash)
    pub batch_id: BatchId,
    /// EVM header context of the last block in this batch.
    pub evm_header: EvmHeaderDigest,
    /// Aggregated state diff for the batch (can be empty for batches with no state changes)
    pub state_diff: BatchStateDiff,
}

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
// Using 395,000 to keep a safe margin.

/// Maximum chunk payload size in bytes.
const MAX_CHUNK_PAYLOAD: usize = 395_000;

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
    blob.chunks(MAX_CHUNK_PAYLOAD).map(|c| c.to_vec()).collect()
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

/// Splits a [`DaBlob`] into encoded DA chunks ready for envelope inscription.
///
/// Encodes the blob using `strata-codec`, then splits the encoded bytes into
/// chunks. Each returned `Vec<u8>` contains a serialized `DaChunkHeader`
/// followed by the chunk payload — the format expected by `decode_da_chunk`.
pub fn prepare_da_chunks(blob: &DaBlob) -> Result<Vec<Vec<u8>>, CodecError> {
    let encoded = encode_to_vec(blob)?;
    let hash = blob_hash(&encoded);
    let payloads = split_blob(&encoded);
    let total_chunks = u16::try_from(payloads.len())
        .map_err(|_| CodecError::MalformedField("blob too large: chunk count exceeds u16::MAX"))?;

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

/// Reassembles a [`DaBlob`] from raw encoded chunks (header ++ payload each).
///
/// Performs the full pipeline: decode headers, validate consistency,
/// order by `chunk_index`, concatenate payloads, verify SHA-256 hash,
/// and decode the resulting bytes into a `DaBlob`.
/// Returns `None` on any validation failure.
pub fn reassemble_da_blob(encoded_chunks: &[Vec<u8>]) -> Option<DaBlob> {
    let bytes = reassemble_from_da_chunks(encoded_chunks)?;
    decode_buf_exact(&bytes).ok()
}

/// Reassembles raw bytes from encoded chunks (header ++ payload each).
///
/// Performs the full pipeline: decode headers, reject unknown versions,
/// order by `chunk_index`, concatenate payloads, compute the blob hash,
/// and verify every chunk's claimed hash against the computed value.
/// Returns `None` on any validation failure.
fn reassemble_from_da_chunks(encoded_chunks: &[Vec<u8>]) -> Option<Vec<u8>> {
    if encoded_chunks.is_empty() {
        return None;
    }

    // Decode all chunks and reject unknown versions.
    let mut decoded: Vec<(DaChunkHeader, &[u8])> = Vec::with_capacity(encoded_chunks.len());
    for enc in encoded_chunks {
        let (header, payload) = decode_da_chunk(enc).ok()?;
        if header.version != DA_CHUNK_ENCODING_VERSION {
            return None;
        }
        decoded.push((header, payload));
    }

    // All chunks must agree on total_chunks count.
    let total_chunks = decoded[0].0.total_chunks;
    if total_chunks as usize != decoded.len() {
        return None;
    }
    if decoded[1..]
        .iter()
        .any(|(h, _)| h.total_chunks != total_chunks)
    {
        return None;
    }

    // Sort by index and verify contiguous [0..total_chunks).
    decoded.sort_by_key(|(h, _)| h.chunk_index);
    for (i, (header, _)) in decoded.iter().enumerate() {
        if header.chunk_index != i as u16 {
            return None;
        }
    }

    // Concatenate payloads, compute hash, verify every chunk's claimed hash.
    let blob: Vec<u8> = decoded
        .iter()
        .flat_map(|(_, p)| p.iter().copied())
        .collect();
    let computed_hash = blob_hash(&blob);
    if decoded.iter().any(|(h, _)| h.blob_hash != computed_hash) {
        return None;
    }

    Some(blob)
}

#[cfg(test)]
mod tests {
    use core::iter::repeat_n;

    use strata_acct_types::Hash;

    use super::*;

    fn make_test_da_blob() -> DaBlob {
        DaBlob {
            batch_id: BatchId::from_parts(Hash::from([0x11; 32]), Hash::from([0x22; 32])),
            evm_header: EvmHeaderDigest {
                block_num: 42,
                timestamp: 1_700_000_000,
                base_fee: 1_000_000_000,
                gas_used: 15_000_000,
                gas_limit: 30_000_000,
            },
            state_diff: BatchStateDiff::default(),
        }
    }

    /// Asserts that two DaBlobs have identical metadata and empty state diffs.
    fn assert_da_blob_eq(a: &DaBlob, b: &DaBlob) {
        assert_eq!(a.batch_id, b.batch_id, "batch_id mismatch");
        assert_eq!(a.evm_header, b.evm_header, "evm_header mismatch");
        assert!(a.state_diff.is_empty(), "expected empty state_diff in a");
        assert!(b.state_diff.is_empty(), "expected empty state_diff in b");
    }

    #[test]
    fn chunk_header_codec_produces_exact_size() {
        let header = DaChunkHeader::new(Buf32::from([0x42; 32]), 3, 10).unwrap();
        let encoded = encode_to_vec(&header).unwrap();
        assert_eq!(encoded.len(), DA_CHUNK_HEADER_SIZE);
        let decoded: DaChunkHeader = decode_buf_exact(&encoded).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn da_blob_codec_roundtrip() {
        let blob = make_test_da_blob();
        let encoded = encode_to_vec(&blob).unwrap();
        let decoded: DaBlob = decode_buf_exact(&encoded).unwrap();
        assert_da_blob_eq(&blob, &decoded);
    }

    #[test]
    fn split_and_reassemble_raw_bytes_roundtrip() {
        for size in [1, 100, MAX_CHUNK_PAYLOAD, MAX_CHUNK_PAYLOAD * 2 + 100] {
            let bytes: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
            let chunks = split_blob(&bytes);
            let reassembled: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
            assert_eq!(reassembled, bytes);
            assert_eq!(blob_hash(&reassembled), blob_hash(&bytes));
        }
    }

    #[test]
    fn full_pipeline_roundtrip() {
        let blob = make_test_da_blob();
        let encoded_chunks = prepare_da_chunks(&blob).unwrap();
        let reassembled = reassemble_da_blob(&encoded_chunks).unwrap();
        assert_da_blob_eq(&blob, &reassembled);
    }

    #[test]
    fn full_pipeline_handles_unordered_input() {
        let blob = make_test_da_blob();
        let mut encoded_chunks = prepare_da_chunks(&blob).unwrap();
        encoded_chunks.reverse();
        let reassembled = reassemble_da_blob(&encoded_chunks).unwrap();
        assert_da_blob_eq(&blob, &reassembled);
    }

    #[test]
    fn full_pipeline_rejects_invalid_input() {
        // Empty input
        assert!(reassemble_da_blob(&[]).is_none());
        // Garbage input
        assert!(reassemble_da_blob(&[vec![0xFF; 10]]).is_none());

        // Missing chunk - test with raw bytes that span multiple chunks
        let large_bytes: Vec<u8> = repeat_n(0u8, MAX_CHUNK_PAYLOAD + 100).collect();
        let hash = blob_hash(&large_bytes);
        let payloads = split_blob(&large_bytes);
        let total = payloads.len() as u16;
        let mut chunks: Vec<Vec<u8>> = payloads
            .iter()
            .enumerate()
            .map(|(i, p)| {
                encode_da_chunk(&DaChunkHeader::new(hash, i as u16, total).unwrap(), p).unwrap()
            })
            .collect();
        chunks.remove(1); // Remove second chunk
        assert!(reassemble_from_da_chunks(&chunks).is_none());
    }
}
