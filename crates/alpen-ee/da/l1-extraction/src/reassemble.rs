//! Reassembles parsed EE DA envelopes into decoded DA blobs.

use alpen_ee_da_types::{reassemble_da_blob, DaBlob};
use strata_codec::CodecError;
use thiserror::Error;

use crate::ParsedEnvelope;

/// Errors raised while reassembling parsed EE DA envelopes.
#[derive(Debug, Error)]
pub enum ReassembleError {
    #[error("blob reassembly failed at envelope {index}: {source}")]
    Envelope {
        index: usize,
        #[source]
        source: CodecError,
    },

    #[error("duplicate DA blob update_seq_no {update_seq_no}")]
    DuplicateUpdateSeqNo { update_seq_no: u64 },

    #[error(
        "expected DA blob update_seq_no {expected} not found in supplied range; next blob has {actual}"
    )]
    MissingUpdateSeqNo { expected: u64, actual: u64 },
}

/// Reassembles parsed envelope chunks into decoded DA blobs.
///
/// The returned blobs are sorted by `update_seq_no`. Duplicate sequence numbers
/// and gaps inside the returned range are rejected.
pub fn reassemble_da_blobs(
    envelopes: impl IntoIterator<Item = ParsedEnvelope>,
) -> Result<Vec<DaBlob>, ReassembleError> {
    let mut blobs = Vec::new();

    for (index, envelope) in envelopes.into_iter().enumerate() {
        let blob = reassemble_da_blob(envelope.chunks())
            .map_err(|source| ReassembleError::Envelope { index, source })?;
        blobs.push(blob);
    }

    blobs.sort_by_key(|blob| blob.update_seq_no);
    reject_update_seq_no_gaps(&blobs)?;

    Ok(blobs)
}

fn reject_update_seq_no_gaps(blobs: &[DaBlob]) -> Result<(), ReassembleError> {
    for pair in blobs.windows(2) {
        let current = pair[0].update_seq_no;
        let next = pair[1].update_seq_no;
        if current == next {
            return Err(ReassembleError::DuplicateUpdateSeqNo {
                update_seq_no: current,
            });
        }

        let expected = current + 1;
        if next != expected {
            return Err(ReassembleError::MissingUpdateSeqNo {
                expected,
                actual: next,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::{encode_to_vec, CodecError};

    use super::{reassemble_da_blobs, ReassembleError};
    use crate::test_utils::{
        build_parsed_envelope_from_chunk_bytes, make_multi_chunk_test_blob, make_test_blob,
        multi_chunk_bytecode_len_strategy,
    };

    const MAX_TEST_CHUNK_BYTES: usize = 395_000;

    fn prepare_test_chunks(blob: &alpen_ee_da_types::DaBlob) -> Vec<Vec<u8>> {
        encode_to_vec(blob)
            .expect("test blob encodes")
            .chunks(MAX_TEST_CHUNK_BYTES)
            .map(|chunk| chunk.to_vec())
            .collect()
    }

    #[test]
    fn reassemble_da_blobs_returns_empty_for_empty_input() {
        let blobs = reassemble_da_blobs(Vec::new()).expect("empty input is valid");
        assert!(blobs.is_empty());
    }

    #[test]
    fn reassemble_da_blobs_sorts_by_update_seq_no() {
        let blob0 = make_test_blob(10);
        let blob1 = make_test_blob(11);
        let envelopes = vec![
            build_parsed_envelope_from_chunk_bytes(prepare_test_chunks(&blob1)),
            build_parsed_envelope_from_chunk_bytes(prepare_test_chunks(&blob0)),
        ];

        let blobs = reassemble_da_blobs(envelopes).expect("reassembly succeeds");

        assert_eq!(
            blobs
                .iter()
                .map(|blob| blob.update_seq_no)
                .collect::<Vec<_>>(),
            vec![10, 11]
        );
    }

    #[test]
    fn reassemble_da_blobs_rejects_duplicate_update_seq_no() {
        let blob0 = make_test_blob(10);
        let blob1 = make_test_blob(10);
        let envelopes = vec![
            build_parsed_envelope_from_chunk_bytes(prepare_test_chunks(&blob0)),
            build_parsed_envelope_from_chunk_bytes(prepare_test_chunks(&blob1)),
        ];

        let err = reassemble_da_blobs(envelopes).expect_err("duplicate seqno must fail");

        assert!(matches!(
            err,
            ReassembleError::DuplicateUpdateSeqNo { update_seq_no: 10 }
        ));
    }

    #[test]
    fn reassemble_da_blobs_rejects_update_seq_no_gap() {
        let blob0 = make_test_blob(10);
        let blob1 = make_test_blob(12);
        let envelopes = vec![
            build_parsed_envelope_from_chunk_bytes(prepare_test_chunks(&blob0)),
            build_parsed_envelope_from_chunk_bytes(prepare_test_chunks(&blob1)),
        ];

        let err = reassemble_da_blobs(envelopes).expect_err("seqno gap must fail");

        assert!(matches!(
            err,
            ReassembleError::MissingUpdateSeqNo {
                expected: 11,
                actual: 12
            }
        ));
    }

    proptest! {
        #[test]
        fn reassemble_da_blobs_reports_failing_envelope_index(
            valid_prefix_len in 0usize..=4,
        ) {
            let mut envelopes = (0..valid_prefix_len)
                .map(|idx| make_test_blob(idx as u64))
                .map(|blob| {
                    build_parsed_envelope_from_chunk_bytes(
                        prepare_test_chunks(&blob),
                    )
                })
                .collect::<Vec<_>>();
            envelopes.push(build_parsed_envelope_from_chunk_bytes(Vec::new()));

            let err = reassemble_da_blobs(envelopes).expect_err("empty envelope must fail");
            match err {
                ReassembleError::Envelope { index, source: CodecError::MalformedField(_) } => {
                    prop_assert_eq!(index, valid_prefix_len);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn reassemble_da_blobs_reassembles_multi_chunk_blob(
            block_num in any::<u64>(),
            bytecode_len in multi_chunk_bytecode_len_strategy(),
            fill_byte in any::<u8>(),
        ) {
            let expected = make_multi_chunk_test_blob(block_num, bytecode_len, fill_byte);
            let chunks = prepare_test_chunks(&expected);
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
