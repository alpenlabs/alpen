//! Replay entry points.

use alpen_ee_da_types::DaBlob;
use alpen_reth_statediff::StateReconstructor;
use strata_identifiers::Buf32;

use crate::{
    error::ReplayError,
    snapshot::ReplayPreStateSnapshot,
    summary::{AppliedExecBlockRange, ReplaySummary},
};

/// Replays genesis-start ordered DA blobs into an in-memory state reconstructor.
///
/// The input must be sorted by [`DaBlob::update_seq_no`] and must start at
/// update sequence number 0 when non-empty. This function initializes state from
/// the supplied chain spec, so final-root reconstruction is meaningful only for
/// a complete replay from the first posted EE DA update. Partial-range replay
/// needs an explicit starting state snapshot, not just a starting root.
///
/// Empty input returns the chain-spec genesis root as the final state root and
/// no applied range.
pub fn replay_blobs_from_genesis(
    chain_spec: &str,
    blobs: &[DaBlob],
) -> Result<ReplaySummary, ReplayError> {
    let reconstructor = StateReconstructor::from_chain_spec(chain_spec).map_err(|error| {
        ReplayError::InvalidChainSpec {
            chain_spec: chain_spec.to_owned(),
            message: error.to_string(),
        }
    })?;

    if let Some(first) = blobs.first() {
        if first.update_seq_no != 0 {
            return Err(ReplayError::NonGenesisStart {
                first_update_seq_no: first.update_seq_no,
            });
        }
    }

    replay_blobs_with_reconstructor(reconstructor, blobs)
}

/// Replays ordered DA blobs from an explicit starting state snapshot.
///
/// The snapshot must represent the EE state immediately before the first
/// supplied blob. The seeded state root is checked before any blob is applied.
/// Empty input returns the snapshot root as the final state root and no applied
/// range.
pub fn replay_blobs_from_snapshot(
    snapshot: &ReplayPreStateSnapshot,
    blobs: &[DaBlob],
) -> Result<ReplaySummary, ReplayError> {
    let reconstructor = StateReconstructor::from_seed(snapshot.state_seed())
        .map_err(|source| ReplayError::InvalidSnapshotState { source })?;
    let actual_state_root = state_root(&reconstructor);
    if actual_state_root != snapshot.expected_state_root() {
        return Err(ReplayError::SnapshotRootMismatch {
            expected_state_root: snapshot.expected_state_root(),
            actual_state_root,
        });
    }

    if let Some(first) = blobs.first() {
        if first.update_seq_no != snapshot.next_update_seq_no() {
            return Err(ReplayError::SnapshotUpdateSeqNoMismatch {
                expected_update_seq_no: snapshot.next_update_seq_no(),
                actual_update_seq_no: first.update_seq_no,
            });
        }

        let first_blob_block_num = first.evm_header.block_num;
        if first_blob_block_num <= snapshot.last_applied_block_num() {
            return Err(ReplayError::SnapshotBlockAnchorMismatch {
                last_applied_block_num: snapshot.last_applied_block_num(),
                first_blob_block_num,
            });
        }
    }

    replay_blobs_with_reconstructor(reconstructor, blobs)
}

fn replay_blobs_with_reconstructor(
    mut reconstructor: StateReconstructor,
    blobs: &[DaBlob],
) -> Result<ReplaySummary, ReplayError> {
    let mut previous_update_seq_no: Option<u64> = None;
    let mut previous_block_num: Option<u64> = None;

    for (blob_index, blob) in blobs.iter().enumerate() {
        if let Some(previous_update_seq_no) = previous_update_seq_no {
            if blob.update_seq_no == previous_update_seq_no {
                return Err(ReplayError::DuplicateUpdateSeqNo {
                    blob_index,
                    update_seq_no: blob.update_seq_no,
                });
            }

            let expected_update_seq_no = previous_update_seq_no.saturating_add(1);
            if blob.update_seq_no != expected_update_seq_no {
                return Err(ReplayError::UpdateSeqNoGap {
                    blob_index,
                    expected_update_seq_no,
                    actual_update_seq_no: blob.update_seq_no,
                });
            }
        }

        let current_block_num = blob.evm_header.block_num;
        if let Some(previous_block_num) = previous_block_num {
            if current_block_num <= previous_block_num {
                return Err(ReplayError::NonIncreasingBlockNumber {
                    blob_index,
                    previous_block_num,
                    current_block_num,
                });
            }
        }

        reconstructor
            .apply_diff(&blob.state_diff)
            .map_err(|source| ReplayError::ApplyDiff { blob_index, source })?;

        previous_update_seq_no = Some(blob.update_seq_no);
        previous_block_num = Some(current_block_num);
    }

    let final_state_root = state_root(&reconstructor);
    let final_state_seed = reconstructor.to_seed();
    let applied = match (blobs.first(), blobs.last()) {
        (Some(first), Some(last)) => Some(AppliedExecBlockRange::new(first, last, blobs.len())),
        _ => None,
    };

    Ok(ReplaySummary::new(
        applied,
        final_state_root,
        final_state_seed,
    ))
}

fn state_root(reconstructor: &StateReconstructor) -> Buf32 {
    let state_root_bytes: [u8; 32] = reconstructor.state_root().into();
    Buf32::from(state_root_bytes)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use alpen_ee_da_types::{DaBlob, EvmHeaderSummary};
    use alpen_reth_statediff::{StateReconstructor, StateReconstructorSeed};
    use proptest::prelude::*;
    use strata_identifiers::Buf32;

    use crate::{
        replay_blobs_from_genesis, replay_blobs_from_snapshot, ReplayError, ReplayPreStateSnapshot,
    };

    const TEST_CHAIN_SPEC: &str = "dev";
    const MAX_SEQUENCE_INCREMENT_SUM: u64 = 16;
    const MAX_SEQUENCE_LEN: usize = 8;

    #[test]
    fn replay_blobs_from_genesis_returns_empty_summary_for_empty_input() {
        let summary = replay_blobs_from_genesis(TEST_CHAIN_SPEC, &[]).expect("replay must succeed");
        assert_eq!(summary.applied(), None);
    }

    #[test]
    fn replay_blobs_from_snapshot_returns_snapshot_root_for_empty_input() {
        let snapshot = test_snapshot(10, 100);

        let summary = replay_blobs_from_snapshot(&snapshot, &[]).expect("replay must succeed");

        assert_eq!(summary.applied(), None);
        assert_eq!(summary.final_state_root(), snapshot.expected_state_root());
        assert_eq!(summary.final_state_seed(), snapshot.state_seed());
    }

    #[test]
    fn replay_blobs_from_snapshot_applies_valid_partial_sequence() {
        let snapshot = test_snapshot(5, 99);
        let blobs = vec![test_blob(5, 100), test_blob(6, 101)];

        let summary = replay_blobs_from_snapshot(&snapshot, &blobs).expect("replay must succeed");
        let applied = summary.applied().expect("applied range must be populated");

        assert_eq!(applied.count(), 2);
        assert_eq!(applied.first_update_seq_no(), 5);
        assert_eq!(applied.last_update_seq_no(), 6);
        assert_eq!(applied.first_block_num(), 100);
        assert_eq!(applied.last_block_num(), 101);
    }

    #[test]
    fn replay_blobs_from_snapshot_rejects_root_mismatch() {
        let snapshot = ReplayPreStateSnapshot::new(
            Buf32::from([0x42; 32]),
            5,
            99,
            StateReconstructorSeed::default(),
            BTreeMap::new(),
        );

        let err =
            replay_blobs_from_snapshot(&snapshot, &[]).expect_err("replay must reject snapshot");

        match err {
            ReplayError::SnapshotRootMismatch {
                expected_state_root,
                ..
            } => assert_eq!(expected_state_root, snapshot.expected_state_root()),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn replay_blobs_from_snapshot_rejects_update_seq_no_mismatch() {
        let snapshot = test_snapshot(5, 99);
        let blobs = vec![test_blob(6, 100)];

        let err =
            replay_blobs_from_snapshot(&snapshot, &blobs).expect_err("replay must reject anchor");

        match err {
            ReplayError::SnapshotUpdateSeqNoMismatch {
                expected_update_seq_no,
                actual_update_seq_no,
            } => {
                assert_eq!(expected_update_seq_no, 5);
                assert_eq!(actual_update_seq_no, 6);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn replay_blobs_from_snapshot_rejects_block_anchor_mismatch() {
        let snapshot = test_snapshot(5, 100);
        let blobs = vec![test_blob(5, 100)];

        let err =
            replay_blobs_from_snapshot(&snapshot, &blobs).expect_err("replay must reject anchor");

        match err {
            ReplayError::SnapshotBlockAnchorMismatch {
                last_applied_block_num,
                first_blob_block_num,
            } => {
                assert_eq!(last_applied_block_num, 100);
                assert_eq!(first_blob_block_num, 100);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn replay_blobs_from_snapshot_rejects_update_seq_no_gap_after_anchor() {
        let snapshot = test_snapshot(5, 99);
        let blobs = vec![test_blob(5, 100), test_blob(7, 101)];

        let err =
            replay_blobs_from_snapshot(&snapshot, &blobs).expect_err("replay must reject gap");

        match err {
            ReplayError::UpdateSeqNoGap {
                blob_index,
                expected_update_seq_no,
                actual_update_seq_no,
            } => {
                assert_eq!(blob_index, 1);
                assert_eq!(expected_update_seq_no, 6);
                assert_eq!(actual_update_seq_no, 7);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    proptest! {
        #[test]
        fn replay_blobs_from_genesis_applies_valid_contiguous_sequence(
            first_block_num in 0u64..=(u64::MAX - MAX_SEQUENCE_INCREMENT_SUM),
            count in 1usize..=MAX_SEQUENCE_LEN,
        ) {
            let blobs = make_contiguous_blobs(first_block_num, count);

            let summary = replay_blobs_from_genesis(TEST_CHAIN_SPEC, &blobs).expect("replay must succeed");
            let applied = summary.applied().expect("applied range must be populated");
            prop_assert_eq!(applied.count(), count);
            prop_assert_eq!(applied.first_update_seq_no(), 0);
            prop_assert_eq!(applied.last_update_seq_no(), count as u64 - 1);
            prop_assert_eq!(applied.first_block_num(), first_block_num);
            prop_assert_eq!(applied.last_block_num(), first_block_num + count as u64 - 1);
        }

        #[test]
        fn replay_blobs_from_genesis_rejects_non_genesis_start(
            first_update_seq_no in 1u64..=u64::MAX,
            first_block_num in any::<u64>(),
        ) {
            let blobs = vec![test_blob(first_update_seq_no, first_block_num)];

            let err = replay_blobs_from_genesis(TEST_CHAIN_SPEC, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::NonGenesisStart { first_update_seq_no: got } => {
                    prop_assert_eq!(got, first_update_seq_no);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn replay_blobs_from_genesis_rejects_update_seq_no_gap(
            first_block_num in 0u64..=(u64::MAX - 2),
        ) {
            let blobs = vec![
                test_blob(0, first_block_num),
                test_blob(2, first_block_num + 1),
            ];

            let err = replay_blobs_from_genesis(TEST_CHAIN_SPEC, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::UpdateSeqNoGap { blob_index, expected_update_seq_no, actual_update_seq_no } => {
                    prop_assert_eq!(blob_index, 1);
                    prop_assert_eq!(expected_update_seq_no, 1);
                    prop_assert_eq!(actual_update_seq_no, 2);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn replay_blobs_from_genesis_rejects_duplicate_update_seq_no(
            first_block_num in 0u64..=(u64::MAX - 2),
        ) {
            let blobs = vec![
                test_blob(0, first_block_num),
                test_blob(0, first_block_num + 1),
            ];

            let err = replay_blobs_from_genesis(TEST_CHAIN_SPEC, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::DuplicateUpdateSeqNo { blob_index, update_seq_no: got } => {
                    prop_assert_eq!(blob_index, 1);
                    prop_assert_eq!(got, 0);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn replay_blobs_from_genesis_rejects_non_increasing_block_number(
            first_block_num in any::<u64>(),
            non_increase in 0u64..=3,
        ) {
            let second_block_num = first_block_num.saturating_sub(non_increase);
            let blobs = vec![
                test_blob(0, first_block_num),
                test_blob(1, second_block_num),
            ];

            let err = replay_blobs_from_genesis(TEST_CHAIN_SPEC, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::NonIncreasingBlockNumber { blob_index, .. } => {
                    prop_assert_eq!(blob_index, 1);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }
    }

    fn test_blob(update_seq_no: u64, block_num: u64) -> DaBlob {
        let gas_used = block_num % 1_000;
        DaBlob {
            update_seq_no,
            evm_header: EvmHeaderSummary {
                block_num,
                timestamp: block_num,
                base_fee: block_num,
                gas_used,
                gas_limit: gas_used.saturating_add(1),
            },
            state_diff: Default::default(),
        }
    }

    fn make_contiguous_blobs(first_block_num: u64, count: usize) -> Vec<DaBlob> {
        (0..count)
            .map(|idx| test_blob(idx as u64, first_block_num + idx as u64))
            .collect()
    }

    fn test_snapshot(
        next_update_seq_no: u64,
        last_applied_block_num: u64,
    ) -> ReplayPreStateSnapshot {
        let state_seed = StateReconstructorSeed::default();
        ReplayPreStateSnapshot::new(
            seed_state_root(&state_seed),
            next_update_seq_no,
            last_applied_block_num,
            state_seed,
            BTreeMap::new(),
        )
    }

    fn seed_state_root(state_seed: &StateReconstructorSeed) -> Buf32 {
        let reconstructor = StateReconstructor::from_seed(state_seed).expect("seed must be valid");
        let root: [u8; 32] = reconstructor.state_root().into();
        Buf32::from(root)
    }
}
