//! Reassembles per-envelope chunk groups into DA blobs.

use alpen_ee_common::{reassemble_da_blob, DaBlob, ReassemblyError};
use thiserror::Error;

use crate::l1::scan::RevealRecord;

/// Errors raised while reassembling segmented chunk groups into DA blobs.
#[derive(Debug, Error)]
pub(crate) enum ReassembleError {
    #[error("blob at index {blob_index} failed reassembly: {source}")]
    BlobReassembly {
        blob_index: usize,
        #[source]
        source: ReassemblyError,
    },
}

/// Reassembles segmented reveal groups into decoded DA blobs.
pub(crate) fn reassemble_da_blobs(
    segmented_reveals: Vec<Vec<RevealRecord>>,
) -> Result<Vec<DaBlob>, ReassembleError> {
    let mut blobs = Vec::with_capacity(segmented_reveals.len());

    for (blob_index, reveal_group) in segmented_reveals.into_iter().enumerate() {
        let encoded_chunks = reveal_group
            .into_iter()
            .map(|reveal| reveal.chunk_bytes)
            .collect::<Vec<_>>();

        let blob = reassemble_da_blob(&encoded_chunks)
            .map_err(|source| ReassembleError::BlobReassembly { blob_index, source })?;

        blobs.push(blob);
    }

    Ok(blobs)
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{prepare_da_chunks, ReassemblyError};
    use proptest::{collection, prelude::*};

    use super::{reassemble_da_blobs, ReassembleError};
    use crate::da::test_utils::{
        build_reveal_records_from_chunk_bytes, make_multi_chunk_test_blob, make_test_blob,
        multi_chunk_bytecode_len_strategy,
    };

    #[test]
    fn reassemble_da_blobs_returns_empty_for_empty_input() {
        let blobs = reassemble_da_blobs(Vec::new()).expect("empty input is valid");
        assert!(blobs.is_empty());
    }

    proptest! {
        #[test]
        fn reassemble_da_blobs_preserves_blob_order(
            block_nums in collection::vec(any::<u64>(), 1..=6),
        ) {
            let expected_blobs = block_nums.iter().copied().map(make_test_blob).collect::<Vec<_>>();
            let segmented = expected_blobs
                .iter()
                .map(|blob| {
                    build_reveal_records_from_chunk_bytes(
                        prepare_da_chunks(blob).expect("chunk preparation succeeds"),
                    )
                })
                .collect::<Vec<_>>();

            let blobs = reassemble_da_blobs(segmented).expect("reassembly succeeds");
            prop_assert_eq!(blobs.len(), expected_blobs.len());
            prop_assert_eq!(
                blobs.iter().map(|blob| blob.batch_id).collect::<Vec<_>>(),
                expected_blobs
                    .iter()
                    .map(|blob| blob.batch_id)
                    .collect::<Vec<_>>()
            );
        }

        #[test]
        fn reassemble_da_blobs_reports_failing_blob_index(
            valid_prefix_block_nums in collection::vec(any::<u64>(), 0..=4),
            malformed_block_num in any::<u64>(),
        ) {
            let malformed = make_test_blob(malformed_block_num);
            let mut malformed_group = build_reveal_records_from_chunk_bytes(
                prepare_da_chunks(&malformed).expect("chunk preparation succeeds"),
            );
            malformed_group[0].chunk_bytes.clear();

            let mut segmented = valid_prefix_block_nums
                .iter()
                .copied()
                .map(make_test_blob)
                .map(|blob| {
                    build_reveal_records_from_chunk_bytes(
                        prepare_da_chunks(&blob).expect("chunk preparation succeeds"),
                    )
                })
                .collect::<Vec<_>>();
            segmented.push(malformed_group);

            let err = reassemble_da_blobs(segmented).expect_err("second blob must fail");
            let blob_index = match err {
                ReassembleError::BlobReassembly { blob_index, .. } => blob_index,
            };
            prop_assert_eq!(blob_index, valid_prefix_block_nums.len());
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

            let segmented = vec![build_reveal_records_from_chunk_bytes(chunks)];
            let blobs = reassemble_da_blobs(segmented).expect("reassembly succeeds");
            prop_assert_eq!(blobs.len(), 1);
            let actual_encoded =
                strata_codec::encode_to_vec(&blobs[0]).expect("encode reassembled blob");
            let expected_encoded =
                strata_codec::encode_to_vec(&expected).expect("encode expected blob");
            prop_assert_eq!(actual_encoded, expected_encoded);
        }

        #[test]
        fn reassemble_da_blobs_reports_empty_group(
            valid_prefix_block_nums in collection::vec(any::<u64>(), 0..=4),
        ) {
            let mut segmented = valid_prefix_block_nums
                .iter()
                .copied()
                .map(make_test_blob)
                .map(|blob| {
                    build_reveal_records_from_chunk_bytes(
                        prepare_da_chunks(&blob).expect("chunk preparation succeeds"),
                    )
                })
                .collect::<Vec<_>>();
            segmented.push(Vec::new());

            let err = reassemble_da_blobs(segmented).expect_err("must fail");
            match err {
                ReassembleError::BlobReassembly {
                    blob_index,
                    source: ReassemblyError::Empty,
                } => prop_assert_eq!(blob_index, valid_prefix_block_nums.len()),
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }
    }
}
