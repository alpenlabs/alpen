//! Orders reveals along the single global `prev_wtxid` chain across envelopes.

use std::collections::{HashMap, HashSet};

use bitcoin::{hashes::Hash as _, Wtxid};
use thiserror::Error;

use crate::l1::scan::RevealRecord;

/// DA-genesis predecessor marker used by the writer.
const DA_GENESIS_PREV_WTXID: [u8; 32] = [0u8; 32];

/// Errors raised while ordering reveal records by predecessor linkage.
#[derive(Debug, Error)]
pub(crate) enum WalkError {
    #[error("missing da-genesis start reveal in scanned window")]
    MissingGenesisStart,

    #[error("multiple da-genesis start reveals in scanned window: {count}")]
    MultipleGenesisStarts { count: usize },

    #[error("reveal {wtxid} references missing predecessor {prev_wtxid}")]
    MissingPredecessor { wtxid: Wtxid, prev_wtxid: Wtxid },

    #[error("predecessor {prev_wtxid} has multiple successors")]
    MultipleSuccessors { prev_wtxid: Wtxid },

    #[error("duplicate reveal wtxid in scanned window: {wtxid}")]
    DuplicateWtxid { wtxid: Wtxid },

    #[error("detected cycle at reveal {wtxid}")]
    CycleDetected { wtxid: Wtxid },

    #[error("reveal walk disconnected; {remaining} reveals were not visited")]
    Disconnected { remaining: usize },
}

/// Walks the global reveal chain and returns records in predecessor order.
pub(crate) fn walk_reveals(reveals: Vec<RevealRecord>) -> Result<Vec<RevealRecord>, WalkError> {
    if reveals.is_empty() {
        return Ok(Vec::new());
    }

    let mut by_wtxid = HashMap::with_capacity(reveals.len());
    let mut successors_by_prev: HashMap<[u8; 32], Vec<Wtxid>> = HashMap::new();
    let mut genesis_starts = Vec::new();

    for reveal in reveals {
        let wtxid = reveal.wtxid;
        let prev = reveal.prev_wtxid;

        if by_wtxid.insert(wtxid, reveal).is_some() {
            return Err(WalkError::DuplicateWtxid { wtxid });
        }

        if prev == DA_GENESIS_PREV_WTXID {
            genesis_starts.push(wtxid);
        } else {
            successors_by_prev.entry(prev).or_default().push(wtxid);
        }
    }

    if genesis_starts.is_empty() {
        return Err(WalkError::MissingGenesisStart);
    }
    if genesis_starts.len() > 1 {
        return Err(WalkError::MultipleGenesisStarts {
            count: genesis_starts.len(),
        });
    }

    for (prev, successors) in &successors_by_prev {
        if successors.len() > 1 {
            return Err(WalkError::MultipleSuccessors {
                prev_wtxid: Wtxid::from_byte_array(*prev),
            });
        }

        let prev_wtxid = Wtxid::from_byte_array(*prev);
        if !by_wtxid.contains_key(&prev_wtxid) {
            let successor = successors[0];
            return Err(WalkError::MissingPredecessor {
                wtxid: successor,
                prev_wtxid,
            });
        }
    }

    let total = by_wtxid.len();
    let mut ordered = Vec::with_capacity(total);
    let mut visited = HashSet::with_capacity(total);
    let mut cursor = genesis_starts[0];

    loop {
        if !visited.insert(cursor) {
            // Defensive guard: with unique-wtxid + single-successor invariants,
            // cycles should already be impossible.
            return Err(WalkError::CycleDetected { wtxid: cursor });
        }

        let Some(current) = by_wtxid.remove(&cursor) else {
            return Err(WalkError::Disconnected {
                remaining: total.saturating_sub(ordered.len()),
            });
        };
        ordered.push(current);

        if ordered.len() == total {
            break;
        }

        let Some(next) = successors_by_prev
            .get(&cursor.to_byte_array())
            .and_then(|entries| entries.first())
            .copied()
        else {
            return Err(WalkError::Disconnected {
                remaining: total.saturating_sub(ordered.len()),
            });
        };

        cursor = next;
    }

    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash as _;
    use proptest::{collection, prelude::*};

    use super::{walk_reveals, WalkError};
    use crate::{
        l1::scan::RevealRecord,
        test_utils::{build_reveal_record, chunk_body_strategy, valid_chunk_header_strategy},
    };

    fn build_linear_reveals(parts: &[([u8; 32], u16, u16, Vec<u8>)]) -> Vec<RevealRecord> {
        let mut prev = [0u8; 32];
        let mut reveals = Vec::new();

        for (idx, (blob_hash, chunk_index, total_chunks, body)) in parts.iter().enumerate() {
            let mut wtxid_bytes = [0u8; 32];
            wtxid_bytes[31] = (idx as u8).saturating_add(1);
            let reveal = build_reveal_record(
                wtxid_bytes,
                prev,
                *blob_hash,
                *chunk_index,
                *total_chunks,
                body,
                idx,
            );
            prev = wtxid_bytes;
            reveals.push(reveal);
        }

        reveals
    }

    prop_compose! {
        fn reveal_part_strategy()
            (
                (blob_hash, chunk_index, total_chunks) in valid_chunk_header_strategy(),
                body in chunk_body_strategy(32),
            ) -> ([u8; 32], u16, u16, Vec<u8>) {
                (blob_hash, chunk_index, total_chunks, body)
            }
    }

    proptest! {
        #[test]
        fn walk_reveals_orders_linear_chain(
            parts in collection::vec(reveal_part_strategy(), 1..=16),
        ) {
            let reveals = build_linear_reveals(&parts);
            let ordered = walk_reveals(reveals).expect("walk succeeds");

            prop_assert_eq!(ordered.len(), parts.len());
            prop_assert_eq!(ordered[0].prev_wtxid, [0u8; 32]);
            for idx in 1..ordered.len() {
                prop_assert_eq!(ordered[idx].prev_wtxid, ordered[idx - 1].wtxid.to_byte_array());
            }
        }

        #[test]
        fn walk_reveals_rejects_missing_genesis_start(
            first_prev in any::<[u8; 32]>(),
            (blob_hash, chunk_index, total_chunks) in valid_chunk_header_strategy(),
            body in chunk_body_strategy(16),
        ) {
            prop_assume!(first_prev != [0u8; 32]);
            let tx = build_reveal_record(
                [1u8; 32],
                first_prev,
                blob_hash,
                chunk_index,
                total_chunks,
                &body,
                0,
            );
            let reveals = vec![tx];

            let err = walk_reveals(reveals).expect_err("walk must fail");
            prop_assert!(matches!(err, WalkError::MissingGenesisStart));
        }

        #[test]
        fn walk_reveals_rejects_multiple_genesis_starts(
            header0 in valid_chunk_header_strategy(),
            header1 in valid_chunk_header_strategy(),
            body0 in chunk_body_strategy(16),
            body1 in chunk_body_strategy(16),
        ) {
            let (blob_hash0, chunk_index0, total_chunks0) = header0;
            let (blob_hash1, chunk_index1, total_chunks1) = header1;
            let tx0 = build_reveal_record(
                [1u8; 32],
                [0u8; 32],
                blob_hash0,
                chunk_index0,
                total_chunks0,
                &body0,
                0,
            );
            let tx1 = build_reveal_record(
                [2u8; 32],
                [0u8; 32],
                blob_hash1,
                chunk_index1,
                total_chunks1,
                &body1,
                1,
            );
            let reveals = vec![tx0, tx1];

            let err = walk_reveals(reveals).expect_err("walk must fail");
            let is_multiple_genesis_starts =
                matches!(err, WalkError::MultipleGenesisStarts { count } if count == 2);
            prop_assert!(is_multiple_genesis_starts);
        }

        #[test]
        fn walk_reveals_rejects_missing_predecessor(
            header0 in valid_chunk_header_strategy(),
            header1 in valid_chunk_header_strategy(),
            body0 in chunk_body_strategy(16),
            body1 in chunk_body_strategy(16),
            prev in any::<[u8; 32]>(),
        ) {
            let (blob_hash0, chunk_index0, total_chunks0) = header0;
            let (blob_hash1, chunk_index1, total_chunks1) = header1;
            prop_assume!(prev != [0u8; 32]);
            let tx0_wtxid = [1u8; 32];
            prop_assume!(prev != tx0_wtxid);
            let tx0 = build_reveal_record(
                tx0_wtxid,
                [0u8; 32],
                blob_hash0,
                chunk_index0,
                total_chunks0,
                &body0,
                0,
            );
            let tx1 = build_reveal_record(
                [2u8; 32],
                prev,
                blob_hash1,
                chunk_index1,
                total_chunks1,
                &body1,
                1,
            );
            let reveals = vec![tx0, tx1];

            let err = walk_reveals(reveals).expect_err("walk must fail");
            let is_missing_predecessor = matches!(err, WalkError::MissingPredecessor { .. });
            prop_assert!(is_missing_predecessor);
        }

        #[test]
        fn walk_reveals_rejects_multiple_successors(
            header0 in valid_chunk_header_strategy(),
            header1 in valid_chunk_header_strategy(),
            header2 in valid_chunk_header_strategy(),
            body0 in chunk_body_strategy(16),
            body1 in chunk_body_strategy(16),
            body2 in chunk_body_strategy(16),
        ) {
            let (blob_hash0, chunk_index0, total_chunks0) = header0;
            let (blob_hash1, chunk_index1, total_chunks1) = header1;
            let (blob_hash2, chunk_index2, total_chunks2) = header2;
            let tx0_wtxid = [1u8; 32];
            let tx0 = build_reveal_record(
                tx0_wtxid,
                [0u8; 32],
                blob_hash0,
                chunk_index0,
                total_chunks0,
                &body0,
                0,
            );

            let tx1 = build_reveal_record(
                [2u8; 32],
                tx0_wtxid,
                blob_hash1,
                chunk_index1,
                total_chunks1,
                &body1,
                1,
            );
            let tx2 = build_reveal_record(
                [3u8; 32],
                tx0_wtxid,
                blob_hash2,
                chunk_index2,
                total_chunks2,
                &body2,
                2,
            );
            let reveals = vec![tx0, tx1, tx2];

            let err = walk_reveals(reveals).expect_err("walk must fail");
            let is_multiple_successors = matches!(err, WalkError::MultipleSuccessors { .. });
            prop_assert!(is_multiple_successors);
        }

        #[test]
        fn walk_reveals_rejects_duplicate_wtxid(
            header0 in valid_chunk_header_strategy(),
            header1 in valid_chunk_header_strategy(),
            body0 in chunk_body_strategy(16),
            body1 in chunk_body_strategy(16),
            duplicate_wtxid in any::<[u8; 32]>(),
        ) {
            let (blob_hash0, chunk_index0, total_chunks0) = header0;
            let (blob_hash1, chunk_index1, total_chunks1) = header1;
            let tx0 = build_reveal_record(
                duplicate_wtxid,
                [0u8; 32],
                blob_hash0,
                chunk_index0,
                total_chunks0,
                &body0,
                0,
            );
            let tx1 = build_reveal_record(
                duplicate_wtxid,
                [9u8; 32],
                blob_hash1,
                chunk_index1,
                total_chunks1,
                &body1,
                1,
            );
            let reveals = vec![tx0, tx1];

            let err = walk_reveals(reveals).expect_err("walk must fail");
            let is_duplicate_wtxid = matches!(err, WalkError::DuplicateWtxid { .. });
            prop_assert!(is_duplicate_wtxid);
        }
    }

    #[test]
    fn walk_reveals_detects_disconnected_component() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let c = [3u8; 32];
        let body = [0xCD];

        // Start chain is A->B. Reveal C is self-linked and disconnected from start.
        let reveals = vec![
            build_reveal_record(a, [0u8; 32], [21u8; 32], 0, 1, &body, 0),
            build_reveal_record(b, a, [22u8; 32], 0, 1, &body, 1),
            build_reveal_record(c, c, [23u8; 32], 0, 1, &body, 2),
        ];

        let err = walk_reveals(reveals).expect_err("disconnected segment must fail");
        assert!(matches!(err, WalkError::Disconnected { .. }));
    }
}
