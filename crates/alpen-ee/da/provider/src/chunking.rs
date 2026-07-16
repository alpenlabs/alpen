//! Producer-side helpers for splitting a [`DaBlob`] into envelope-sized chunks.
//!
//! Consumers (proof verifier, host witness builder) only need the codec types
//! and `reassemble_da_blob` from [`alpen_ee_da_types`]; the chunking primitives
//! here are exclusively used by the chunked-envelope DA provider when building
//! inscriptions.

use alpen_ee_da_types::DaBlob;
use strata_codec::{encode_to_vec, CodecError};

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
pub(crate) const MAX_CHUNK_PAYLOAD: usize = 395_000;

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
    // NOTE: add an explicit BlobTooLarge error if batch sizing approaches
    // thousands of chunks.
    Ok(split_blob(&encoded))
}

#[cfg(test)]
mod tests {
    use alpen_ee_da_types::{reassemble_da_blob, DaBlob, EvmHeaderSummary};
    use alpen_reth_statediff::BatchStateDiff;
    use strata_codec::{decode_buf_exact, encode_to_vec};
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
                gas_limit: 36_000_000,
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
}
