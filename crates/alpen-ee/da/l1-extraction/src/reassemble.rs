//! Reassembles parsed EE DA envelopes into decoded DA blobs.
//!
//! Pure codec layer — sequencing (order, gaps, duplicates, anchor) is
//! the caller's responsibility.

use alpen_ee_da_types::{reassemble_da_blob, DaBlob};
use strata_codec::CodecError;
use thiserror::Error;

use crate::ParsedEnvelope;

/// Errors raised while reassembling parsed EE DA envelopes.
#[derive(Debug, Error)]
pub enum ReassembleError {
    #[error("failed to decode DA blob at envelope {index}: {source}")]
    DecodeBlob {
        index: usize,
        #[source]
        source: CodecError,
    },
}

/// Reassembles parsed envelope chunks into decoded DA blobs.
///
/// The returned blobs preserve the supplied envelope order. Sequence
/// validation is handled by state replay.
pub fn reassemble_da_blobs(
    envelopes: impl IntoIterator<Item = ParsedEnvelope>,
) -> Result<Vec<DaBlob>, ReassembleError> {
    let mut blobs = Vec::new();

    for (index, envelope) in envelopes.into_iter().enumerate() {
        let blob = reassemble_da_blob(envelope.chunks())
            .map_err(|source| ReassembleError::DecodeBlob { index, source })?;
        blobs.push(blob);
    }

    Ok(blobs)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::{encode_to_vec, CodecError};

    use super::{reassemble_da_blobs, ReassembleError};
    use crate::test_utils::{
        build_da_blob, build_multi_chunk_da_blob, build_parsed_envelope_from_chunk_bytes,
        multi_chunk_bytecode_len_strategy,
    };

    const MAX_TEST_CHUNK_BYTES: usize = 395_000;

    fn encode_blob_chunks(blob: &alpen_ee_da_types::DaBlob) -> Vec<Vec<u8>> {
        encode_to_vec(blob)
            .expect("DA blob encodes")
            .chunks(MAX_TEST_CHUNK_BYTES)
            .map(|chunk| chunk.to_vec())
            .collect()
    }

    #[test]
    fn test_empty_input() {
        let blobs = reassemble_da_blobs(Vec::new()).expect("empty input is valid");
        assert!(blobs.is_empty());
    }

    #[test]
    fn test_input_order_preserved() {
        let blob0 = build_da_blob(10);
        let blob1 = build_da_blob(11);
        let envelopes = vec![
            build_parsed_envelope_from_chunk_bytes(encode_blob_chunks(&blob1)),
            build_parsed_envelope_from_chunk_bytes(encode_blob_chunks(&blob0)),
        ];

        let blobs = reassemble_da_blobs(envelopes).expect("reassembly succeeds");

        assert_eq!(
            blobs
                .iter()
                .map(|blob| blob.update_seq_no)
                .collect::<Vec<_>>(),
            vec![11, 10]
        );
    }

    proptest! {
        #[test]
        fn test_decode_failure_index(
            valid_prefix_len in 0usize..=4,
        ) {
            let mut envelopes = (0..valid_prefix_len)
                .map(|idx| build_da_blob(idx as u64))
                .map(|blob| {
                    build_parsed_envelope_from_chunk_bytes(
                        encode_blob_chunks(&blob),
                    )
                })
                .collect::<Vec<_>>();
            envelopes.push(build_parsed_envelope_from_chunk_bytes(Vec::new()));

            let err = reassemble_da_blobs(envelopes).expect_err("empty envelope must fail");
            match err {
                ReassembleError::DecodeBlob { index, source: CodecError::MalformedField(_) } => {
                    prop_assert_eq!(index, valid_prefix_len);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn test_multi_chunk_blob(
            block_num in any::<u64>(),
            bytecode_len in multi_chunk_bytecode_len_strategy(),
            fill_byte in any::<u8>(),
        ) {
            let expected = build_multi_chunk_da_blob(block_num, bytecode_len, fill_byte);
            let chunks = encode_blob_chunks(&expected);
            prop_assert!(chunks.len() > 1, "fixture must produce multiple chunks");

            let envelopes = vec![build_parsed_envelope_from_chunk_bytes(chunks)];
            let blobs = reassemble_da_blobs(envelopes).expect("reassembly succeeds");
            prop_assert_eq!(blobs.len(), 1);

            let actual_encoded =
                strata_codec::encode_to_vec(&blobs[0]).expect("encode reassembled blob");
            let expected_encoded =
                strata_codec::encode_to_vec(&expected).expect("encode expected blob");
            prop_assert_eq!(actual_encoded, expected_encoded);
        }
    }
}
