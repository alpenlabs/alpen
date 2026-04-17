//! Reassembles parsed EE DA envelopes into decoded DA blobs.

use alpen_ee_common::{reassemble_da_blob, DaBlob};
use strata_codec::CodecError;
use thiserror::Error;

use crate::ParsedEnvelope;

/// Errors raised while reassembling parsed EE DA envelopes.
#[derive(Debug, Error)]
pub enum ReassembleError {
    #[error("blob reassembly failed at envelope {0}: {1}")]
    Envelope(usize, #[source] CodecError),
}

/// Reassembles parsed envelope chunks into decoded DA blobs.
pub fn reassemble_da_blobs(
    envelopes: impl IntoIterator<Item = ParsedEnvelope>,
) -> Result<Vec<DaBlob>, ReassembleError> {
    let mut blobs = Vec::new();

    for (index, envelope) in envelopes.into_iter().enumerate() {
        let blob = reassemble_da_blob(envelope.chunks())
            .map_err(|source| ReassembleError::Envelope(index, source))?;
        blobs.push(blob);
    }

    Ok(blobs)
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::prepare_da_chunks;
    use proptest::{collection, prelude::*};
    use strata_codec::CodecError;

    use super::{reassemble_da_blobs, ReassembleError};
    use crate::test_utils::{
        build_parsed_envelope_from_chunk_bytes, make_multi_chunk_test_blob, make_test_blob,
        multi_chunk_bytecode_len_strategy,
    };

    #[test]
    fn reassemble_da_blobs_returns_empty_for_empty_input() {
        let blobs = reassemble_da_blobs(Vec::new()).expect("empty input is valid");
        assert!(blobs.is_empty());
    }

    proptest! {
        #[test]
        fn reassemble_da_blobs_preserves_envelope_order(
            block_nums in collection::vec(any::<u64>(), 1..=6),
        ) {
            let expected_blobs = block_nums.iter().copied().map(make_test_blob).collect::<Vec<_>>();
            let envelopes = expected_blobs
                .iter()
                .map(|blob| {
                    build_parsed_envelope_from_chunk_bytes(
                        prepare_da_chunks(blob).expect("chunk preparation succeeds"),
                    )
                })
                .collect::<Vec<_>>();

            let blobs = reassemble_da_blobs(envelopes).expect("reassembly succeeds");
            prop_assert_eq!(blobs.len(), expected_blobs.len());
            prop_assert_eq!(
                blobs
                    .iter()
                    .map(|blob| (blob.update_seq_no, blob.evm_header.block_num))
                    .collect::<Vec<_>>(),
                expected_blobs
                    .iter()
                    .map(|blob| (blob.update_seq_no, blob.evm_header.block_num))
                    .collect::<Vec<_>>()
            );
        }

        #[test]
        fn reassemble_da_blobs_reports_failing_envelope_index(
            valid_prefix_block_nums in collection::vec(any::<u64>(), 0..=4),
        ) {
            let mut envelopes = valid_prefix_block_nums
                .iter()
                .copied()
                .map(make_test_blob)
                .map(|blob| {
                    build_parsed_envelope_from_chunk_bytes(
                        prepare_da_chunks(&blob).expect("chunk preparation succeeds"),
                    )
                })
                .collect::<Vec<_>>();
            envelopes.push(build_parsed_envelope_from_chunk_bytes(Vec::new()));

            let err = reassemble_da_blobs(envelopes).expect_err("empty envelope must fail");
            match err {
                ReassembleError::Envelope(index, CodecError::MalformedField(_)) => {
                    prop_assert_eq!(index, valid_prefix_block_nums.len());
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
            let chunks = prepare_da_chunks(&expected).expect("chunk preparation succeeds");
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
