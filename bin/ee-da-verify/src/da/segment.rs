//! Splits ordered reveals into per-envelope groups.

use alpen_ee_common::DaChunkHeader;
use bitcoin::Wtxid;
use strata_identifiers::Buf32;
use thiserror::Error;

use crate::l1::scan::RevealRecord;

#[derive(Debug, Error)]
pub(crate) enum SegmentError {
    #[error("chunk stream starts mid-envelope at reveal {wtxid}: chunk_index={chunk_index}")]
    ChunkStreamStartsMidEnvelope { wtxid: Wtxid, chunk_index: u16 },

    #[error(
        "blob {blob_hash} has non-contiguous chunk index at reveal {wtxid}: expected {expected}, got {actual}"
    )]
    NonContiguousChunkIndex {
        wtxid: Wtxid,
        blob_hash: Buf32,
        expected: u16,
        actual: u16,
    },

    #[error("blob hash mismatch at reveal {wtxid}: expected {expected}, got {actual}")]
    BlobHashMismatch {
        wtxid: Wtxid,
        expected: Buf32,
        actual: Buf32,
    },

    #[error("total_chunks mismatch at reveal {wtxid}: expected {expected}, got {actual}")]
    TotalChunksMismatch {
        wtxid: Wtxid,
        expected: u16,
        actual: u16,
    },

    #[error("chunk stream ended mid-envelope at reveal {wtxid}: total_chunks={total_chunks}, last_chunk_index={last_chunk_index}")]
    ChunkStreamEndsMidEnvelope {
        wtxid: Wtxid,
        total_chunks: u16,
        last_chunk_index: u16,
    },
}

#[derive(Debug)]
struct PendingEnvelope {
    blob_hash: Buf32,
    total_chunks: u16,
    next_expected_index: u16,
    reveals: Vec<RevealRecord>,
}

/// Segments an ordered reveal stream into per-envelope reveal groups.
pub(crate) fn segment_reveals(
    ordered_reveals: Vec<RevealRecord>,
) -> Result<Vec<Vec<RevealRecord>>, SegmentError> {
    let mut segmented = Vec::new();
    let mut pending: Option<PendingEnvelope> = None;

    for reveal in ordered_reveals {
        let header = reveal.chunk_header;
        if let Some(current) = &mut pending {
            validate_reveal_for_pending(&reveal, header, current)?;
            current.reveals.push(reveal);

            if header.chunk_index() + 1 == current.total_chunks {
                let completed = pending.take().expect("pending is set");
                segmented.push(completed.reveals);
            } else {
                current.next_expected_index += 1;
            }
            continue;
        }

        if header.chunk_index() != 0 {
            return Err(SegmentError::ChunkStreamStartsMidEnvelope {
                wtxid: reveal.wtxid,
                chunk_index: header.chunk_index(),
            });
        }

        let blob_hash = header.blob_hash();
        let total_chunks = header.total_chunks();

        let mut reveals = Vec::new();
        reveals.push(reveal);

        if total_chunks == 1 {
            segmented.push(reveals);
        } else {
            pending = Some(PendingEnvelope {
                blob_hash,
                total_chunks,
                next_expected_index: 1,
                reveals,
            });
        }
    }

    if let Some(current) = pending {
        let last_reveal = current
            .reveals
            .last()
            .expect("pending envelope is non-empty");
        return Err(SegmentError::ChunkStreamEndsMidEnvelope {
            wtxid: last_reveal.wtxid,
            total_chunks: current.total_chunks,
            last_chunk_index: last_reveal.chunk_header.chunk_index(),
        });
    }

    Ok(segmented)
}

fn validate_reveal_for_pending(
    reveal: &RevealRecord,
    header: DaChunkHeader,
    current: &PendingEnvelope,
) -> Result<(), SegmentError> {
    let actual_blob_hash = header.blob_hash();

    if actual_blob_hash != current.blob_hash {
        return Err(SegmentError::BlobHashMismatch {
            wtxid: reveal.wtxid,
            expected: current.blob_hash,
            actual: actual_blob_hash,
        });
    }

    if header.total_chunks() != current.total_chunks {
        return Err(SegmentError::TotalChunksMismatch {
            wtxid: reveal.wtxid,
            expected: current.total_chunks,
            actual: header.total_chunks(),
        });
    }

    if header.chunk_index() != current.next_expected_index {
        return Err(SegmentError::NonContiguousChunkIndex {
            wtxid: reveal.wtxid,
            blob_hash: current.blob_hash,
            expected: current.next_expected_index,
            actual: header.chunk_index(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::test_utils::{build_reveal_record, chunk_body_strategy};

    fn distinct_blob_hash_pair_strategy() -> impl Strategy<Value = ([u8; 32], [u8; 32])> {
        (any::<[u8; 32]>(), any::<[u8; 32]>())
            .prop_filter("blob hashes must differ", |(a, b)| a != b)
    }

    fn is_chunk_stream_starts_mid_envelope(err: &SegmentError) -> bool {
        matches!(err, SegmentError::ChunkStreamStartsMidEnvelope { .. })
    }

    fn is_non_contiguous_chunk_index(err: &SegmentError) -> bool {
        matches!(err, SegmentError::NonContiguousChunkIndex { .. })
    }

    fn is_blob_hash_mismatch(err: &SegmentError) -> bool {
        matches!(err, SegmentError::BlobHashMismatch { .. })
    }

    fn is_total_chunks_mismatch(err: &SegmentError) -> bool {
        matches!(err, SegmentError::TotalChunksMismatch { .. })
    }

    fn is_chunk_stream_ends_mid_envelope(err: &SegmentError) -> bool {
        matches!(err, SegmentError::ChunkStreamEndsMidEnvelope { .. })
    }

    #[test]
    fn segment_reveals_returns_empty_for_empty_input() {
        let grouped = segment_reveals(Vec::new()).expect("empty input is valid");
        assert!(grouped.is_empty());
    }

    proptest! {
        #[test]
        fn segment_reveals_reports_blob_hash_mismatch_on_early_new_envelope(
            w0 in any::<[u8; 32]>(),
            w1 in any::<[u8; 32]>(),
            p0 in any::<[u8; 32]>(),
            p1 in any::<[u8; 32]>(),
            hashes in distinct_blob_hash_pair_strategy(),
            body0 in chunk_body_strategy(16),
            body1 in chunk_body_strategy(16),
        ) {
            let (first_blob_hash, second_blob_hash) = hashes;
            // First reveal opens envelope A (idx=0, total=2).
            // Second reveal starts envelope B (idx=0, total=2) before A closes.
            let reveals = vec![
                build_reveal_record(w0, p0, first_blob_hash, 0, 2, &body0, 0),
                build_reveal_record(w1, p1, second_blob_hash, 0, 2, &body1, 1),
            ];

            let err = segment_reveals(reveals).expect_err("must fail");
            prop_assert!(is_blob_hash_mismatch(&err));
        }

        #[test]
        fn segment_reveals_groups_chunks_by_envelope_boundaries(
            blob_hashes in distinct_blob_hash_pair_strategy(),
            w0 in any::<[u8; 32]>(),
            w1 in any::<[u8; 32]>(),
            w2 in any::<[u8; 32]>(),
            p0 in any::<[u8; 32]>(),
            p1 in any::<[u8; 32]>(),
            p2 in any::<[u8; 32]>(),
            body0 in chunk_body_strategy(32),
            body1 in chunk_body_strategy(32),
            body2 in chunk_body_strategy(32),
        ) {
            let (first_blob_hash, second_blob_hash) = blob_hashes;
            let reveals = vec![
                build_reveal_record(w0, p0, first_blob_hash, 0, 2, &body0, 0),
                build_reveal_record(w1, p1, first_blob_hash, 1, 2, &body1, 1),
                build_reveal_record(w2, p2, second_blob_hash, 0, 1, &body2, 2),
            ];

            let grouped = segment_reveals(reveals).expect("segmentation succeeds");
            prop_assert_eq!(grouped.len(), 2);
            prop_assert_eq!(grouped[0].len(), 2);
            prop_assert_eq!(grouped[1].len(), 1);
        }

        #[test]
        fn segment_reveals_rejects_stream_starting_mid_envelope(
            wtxid in any::<[u8; 32]>(),
            prev in any::<[u8; 32]>(),
            blob_hash in any::<[u8; 32]>(),
            total_chunks in 2u16..=u16::MAX,
            chunk_index in 1u16..=u16::MAX,
            body in chunk_body_strategy(16),
        ) {
            prop_assume!(chunk_index < total_chunks);
            let reveals = vec![build_reveal_record(
                wtxid,
                prev,
                blob_hash,
                chunk_index,
                total_chunks,
                &body,
                0,
            )];

            let err = segment_reveals(reveals).expect_err("must fail");
            prop_assert!(is_chunk_stream_starts_mid_envelope(&err));
        }

        #[test]
        fn segment_reveals_rejects_non_contiguous_indices(
            w0 in any::<[u8; 32]>(),
            w1 in any::<[u8; 32]>(),
            p0 in any::<[u8; 32]>(),
            p1 in any::<[u8; 32]>(),
            blob_hash in any::<[u8; 32]>(),
            total_chunks in 3u16..=u16::MAX,
            second_chunk_index in 2u16..=u16::MAX,
            body0 in chunk_body_strategy(16),
            body1 in chunk_body_strategy(16),
        ) {
            prop_assume!(second_chunk_index < total_chunks);
            let reveals = vec![
                build_reveal_record(w0, p0, blob_hash, 0, total_chunks, &body0, 0),
                build_reveal_record(
                    w1,
                    p1,
                    blob_hash,
                    second_chunk_index,
                    total_chunks,
                    &body1,
                    1,
                ),
            ];

            let err = segment_reveals(reveals).expect_err("must fail");
            prop_assert!(is_non_contiguous_chunk_index(&err));
        }

        #[test]
        fn segment_reveals_rejects_blob_hash_mismatch(
            w0 in any::<[u8; 32]>(),
            w1 in any::<[u8; 32]>(),
            p0 in any::<[u8; 32]>(),
            p1 in any::<[u8; 32]>(),
            hashes in distinct_blob_hash_pair_strategy(),
            total_chunks in 2u16..=u16::MAX,
            body0 in chunk_body_strategy(16),
            body1 in chunk_body_strategy(16),
        ) {
            let (first_blob_hash, second_blob_hash) = hashes;
            let reveals = vec![
                build_reveal_record(w0, p0, first_blob_hash, 0, total_chunks, &body0, 0),
                build_reveal_record(w1, p1, second_blob_hash, 1, total_chunks, &body1, 1),
            ];

            let err = segment_reveals(reveals).expect_err("must fail");
            prop_assert!(is_blob_hash_mismatch(&err));
        }

        #[test]
        fn segment_reveals_rejects_total_chunks_mismatch(
            w0 in any::<[u8; 32]>(),
            w1 in any::<[u8; 32]>(),
            p0 in any::<[u8; 32]>(),
            p1 in any::<[u8; 32]>(),
            blob_hash in any::<[u8; 32]>(),
            first_total_chunks in 2u16..=u16::MAX,
            second_total_chunks in 2u16..=u16::MAX,
            body0 in chunk_body_strategy(16),
            body1 in chunk_body_strategy(16),
        ) {
            prop_assume!(first_total_chunks != second_total_chunks);
            let reveals = vec![
                build_reveal_record(w0, p0, blob_hash, 0, first_total_chunks, &body0, 0),
                build_reveal_record(w1, p1, blob_hash, 1, second_total_chunks, &body1, 1),
            ];

            let err = segment_reveals(reveals).expect_err("must fail");
            prop_assert!(is_total_chunks_mismatch(&err));
        }

        #[test]
        fn segment_reveals_rejects_stream_ending_mid_envelope(
            wtxid in any::<[u8; 32]>(),
            prev in any::<[u8; 32]>(),
            blob_hash in any::<[u8; 32]>(),
            total_chunks in 2u16..=u16::MAX,
            body in chunk_body_strategy(16),
        ) {
            let reveals = vec![build_reveal_record(
                wtxid,
                prev,
                blob_hash,
                0,
                total_chunks,
                &body,
                0,
            )];

            let err = segment_reveals(reveals).expect_err("must fail");
            prop_assert!(is_chunk_stream_ends_mid_envelope(&err));
        }
    }
}
