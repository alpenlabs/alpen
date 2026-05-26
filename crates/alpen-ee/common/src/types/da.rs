//! DA encoding primitives for chunked envelope bundles.
//!
//! Types and functions for splitting and reassembling DA blobs into Bitcoin
//! envelope chunks for inscription.

use alpen_reth_statediff::BatchStateDiff;
use strata_codec::{decode_buf_exact, encode_to_vec, Codec, CodecError};

/// Magic bytes in the EE DA commit transaction marker output.
pub const EE_DA_MAGIC_BYTES: [u8; 4] = *b"ALPN";

/// Current EE DA blob encoding version.
///
/// The commit transaction carries this version next to the EE DA magic bytes
/// in OP_RETURN, so L1 scanners can associate reassembled blob bytes with the schema that
/// produced them. The current decoder handles only the present [`DaBlob`]
/// shape; version dispatch can be added when a future blob schema is
/// introduced.
pub const DA_BLOB_VERSION: u32 = 0;

/// Compact summary of the last EVM block header in a batch.
///
/// Captures the subset of the terminal EVM block header a sequencer recovering
/// purely from L1 DA needs to build the next EVM block. A fresh sequencer has
/// the [`BatchStateDiff`] for account/storage changes but **not** the block
/// headers themselves, so these non-derivable header fields fill that gap. The
/// terminal block id and state root are EE account update metadata, not DA
/// blob fields.
///
/// - `base_fee`, `gas_used`, `gas_limit` feed the EIP-1559 base-fee calculation and gas-limit
///   adjustment for the next block.
/// - `timestamp` enforces monotonicity (`next > parent`).
/// - `block_num` identifies where the chain continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Codec)]
pub struct EvmHeaderSummary {
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
/// It wraps the batch state diff with sequencing metadata needed for L1 sync
/// and chain reconstruction.
#[derive(Debug, Clone, Codec)]
pub struct DaBlob {
    /// Monotonic EE account update sequence number for this blob.
    pub update_seq_no: u64,
    /// EVM header context of the last block in this batch.
    pub evm_header: EvmHeaderSummary,
    /// Aggregated state diff for the batch (can be empty for batches with no state changes)
    pub state_diff: BatchStateDiff,
}

// Bitcoin policy caps standard transactions weight at 400,000 wu. A reveal
// transaction spends one commit output and carries the DA chunk in a tapscript
// envelope:
//
//   <sequencer_pk> OP_CHECKSIG OP_FALSE OP_IF <chunk> OP_ENDIF
//
// Keep the chunk payload ceiling aligned with the upstream envelope builder
// limit. The remaining transaction weight covers the input, sequencer output,
// Schnorr signature, control block, script opcodes, and pushdata framing.
/// Maximum size of an encoded DA chunk payload accepted by the envelope builder.
const MAX_CHUNK_PAYLOAD: usize = 395_000;

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

/// Splits a [`DaBlob`] into raw chunk payloads ready for envelope inscription.
///
/// Returns the chunks in order. Chunk index and total are implicit in
/// commit tx output ordering.
pub fn prepare_da_chunks(blob: &DaBlob) -> Result<Vec<Vec<u8>>, CodecError> {
    let encoded = encode_to_vec(blob)?;
    // Each chunk maps to one reveal tx and one P2TR output in the commit tx.
    // Commit-tx standardness gives the practical chunk-count ceiling: a
    // 43-byte P2TR output under Bitcoin Core's 400,000 wu limit leaves room
    // for roughly 2,300 reveal outputs after the input, OP_RETURN marker, and
    // change output. Current EE DA blobs should stay far below that;
    // NOTE: add an explicit BlobTooLarge error if batch sizing approaches thousands of
    // chunks.
    Ok(split_blob(&encoded))
}

/// Reassembles a [`DaBlob`] from raw chunk payloads.
///
/// `chunks` must be in commit-output order.
pub fn reassemble_da_blob(chunks: &[Vec<u8>]) -> Result<DaBlob, CodecError> {
    if chunks.is_empty() {
        return Err(CodecError::MalformedField("no DA chunks provided"));
    }

    let blob: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
    decode_buf_exact(&blob)
}

#[cfg(test)]
mod tests {
    use strata_l1_envelope_fmt::builder::MAX_ENVELOPE_PAYLOAD_SIZE;

    use super::*;

    fn make_test_da_blob() -> DaBlob {
        DaBlob {
            update_seq_no: 42,
            evm_header: EvmHeaderSummary {
                block_num: 42,
                timestamp: 1_700_000_000,
                base_fee: 1_000_000_000,
                gas_used: 15_000_000,
                gas_limit: 30_000_000,
            },
            state_diff: BatchStateDiff::default(),
        }
    }

    fn assert_da_blob_eq(a: &DaBlob, b: &DaBlob) {
        assert_eq!(a.update_seq_no, b.update_seq_no, "update_seq_no mismatch");
        assert_eq!(a.evm_header, b.evm_header, "evm_header mismatch");
        assert!(a.state_diff.is_empty(), "expected empty state_diff in a");
        assert!(b.state_diff.is_empty(), "expected empty state_diff in b");
    }

    #[test]
    fn da_blob_codec_roundtrip() {
        let blob = make_test_da_blob();
        let encoded = encode_to_vec(&blob).unwrap();
        let decoded: DaBlob = decode_buf_exact(&encoded).unwrap();
        assert_da_blob_eq(&blob, &decoded);
    }

    #[test]
    fn full_pipeline_roundtrip() {
        let blob = make_test_da_blob();
        let chunks = prepare_da_chunks(&blob).unwrap();
        let reassembled = reassemble_da_blob(&chunks).unwrap();
        assert_da_blob_eq(&blob, &reassembled);
    }

    #[test]
    fn envelope_payload_limit_matches_builder_constant() {
        assert_eq!(
            MAX_CHUNK_PAYLOAD, MAX_ENVELOPE_PAYLOAD_SIZE,
            "MAX_CHUNK_PAYLOAD drifted from upstream builder constant (l1_envelope_fmt::builder::MAX_ENVELOPE_PAYLOAD_SIZE)"
        );
    }

    #[test]
    fn reassemble_rejects_empty_input() {
        assert!(reassemble_da_blob(&[]).is_err());
    }

    #[test]
    fn reassemble_rejects_truncated_payload() {
        let blob = make_test_da_blob();
        let mut chunks = prepare_da_chunks(&blob).unwrap();
        chunks[0].truncate(4);
        let err = reassemble_da_blob(&chunks).unwrap_err();
        assert!(matches!(err, CodecError::OverrunInput));
    }
}
