//! Replay entry points.

use alpen_ee_da_types::DaBlob;
use alpen_reth_statediff::{
    apply_batch_state_diff_to_ethereum_state, EthereumStateExt, ReconstructError,
};
use rsp_mpt::EthereumState;

use crate::{
    error::ReplayError,
    snapshot::{ReplayStateSnapshot, SNAPSHOT_FORMAT_VERSION},
    summary::{AppliedExecBlockRange, ReplaySummary},
};

/// EVM genesis is block 0; the first block applied from DA is block 1.
const GENESIS_LAST_APPLIED_BLOCK_NUM: u64 = 0;

/// Replays genesis-start ordered DA blobs into an in-memory Ethereum state.
///
/// The input must be sorted by [`DaBlob::update_seq_no`] and must start at
/// update sequence number 0 when non-empty. The caller must pass an Ethereum
/// state initialized from chain-spec genesis; final-root reconstruction is
/// meaningful only for a complete replay from the first posted EE DA update.
/// Partial-range replay needs an explicit state snapshot, not just a
/// state root.
///
/// Empty input returns the supplied genesis state's root as the final state root
/// and no applied range.
pub fn replay_da_blobs_from_genesis(
    state: EthereumState,
    blobs: &[DaBlob],
) -> Result<ReplaySummary, ReplayError> {
    if let Some(first) = blobs.first() {
        if first.update_seq_no != 0 {
            return Err(ReplayError::NonGenesisStart {
                first_update_seq_no: first.update_seq_no,
            });
        }
    }

    replay_da_blobs_from_state(state, GENESIS_LAST_APPLIED_BLOCK_NUM, blobs)
}

/// Replays ordered DA blobs from an explicit state snapshot.
///
/// The snapshot must represent the EE state immediately before the first
/// supplied blob. The state root is checked before any blob is applied.
/// Empty input returns the snapshot root as the final state root and no applied
/// range.
pub fn replay_da_blobs_from_snapshot(
    snapshot: ReplayStateSnapshot,
    blobs: &[DaBlob],
) -> Result<ReplaySummary, ReplayError> {
    if snapshot.version() != SNAPSHOT_FORMAT_VERSION {
        return Err(ReplayError::UnsupportedSnapshotVersion {
            version: snapshot.version(),
            supported_version: SNAPSHOT_FORMAT_VERSION,
        });
    }

    let expected_state_root = snapshot.expected_state_root();
    let next_update_seq_no = snapshot.next_update_seq_no();
    let last_applied_block_num = snapshot.last_applied_block_num();
    let state = snapshot.into_ethereum_state();
    let actual_state_root = state.state_root_buf32();
    if actual_state_root != expected_state_root {
        return Err(ReplayError::SnapshotRootMismatch {
            expected_state_root,
            actual_state_root,
        });
    }

    if let Some(first) = blobs.first() {
        if first.update_seq_no != next_update_seq_no {
            return Err(ReplayError::SnapshotUpdateSeqNoMismatch {
                expected_update_seq_no: next_update_seq_no,
                actual_update_seq_no: first.update_seq_no,
            });
        }

        let first_blob_block_num = first.evm_header.block_num;
        if first_blob_block_num <= last_applied_block_num {
            return Err(ReplayError::SnapshotBlockAnchorMismatch {
                last_applied_block_num,
                first_blob_block_num,
            });
        }
    }

    replay_da_blobs_from_state(state, last_applied_block_num, blobs)
}

fn replay_da_blobs_from_state(
    mut state: EthereumState,
    last_applied_block_num: u64,
    blobs: &[DaBlob],
) -> Result<ReplaySummary, ReplayError> {
    let mut previous_update_seq_no: Option<u64> = None;
    let mut previous_block_num = Some(last_applied_block_num);
    let mut applied_state_roots = Vec::with_capacity(blobs.len());

    for (blob_index, blob) in blobs.iter().enumerate() {
        if let Some(previous_update_seq_no) = previous_update_seq_no {
            if blob.update_seq_no == previous_update_seq_no {
                return Err(ReplayError::DuplicateUpdateSeqNo {
                    blob_index,
                    update_seq_no: blob.update_seq_no,
                });
            }

            let expected_update_seq_no =
                previous_update_seq_no
                    .checked_add(1)
                    .ok_or(ReplayError::TerminalUpdateSeqNo {
                        blob_index,
                        update_seq_no: previous_update_seq_no,
                    })?;
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

        apply_da_blob(&mut state, blob)
            .map_err(|source| ReplayError::ApplyDiff { blob_index, source })?;
        applied_state_roots.push(state.state_root_buf32());

        previous_update_seq_no = Some(blob.update_seq_no);
        previous_block_num = Some(current_block_num);
    }

    let final_state_root = state.state_root_buf32();
    let applied_evm_headers = blobs.iter().map(|blob| blob.evm_header).collect();
    let applied = match (blobs.first(), blobs.last()) {
        (Some(first), Some(last)) => {
            let Some(first_block_num) = last_applied_block_num.checked_add(1) else {
                unreachable!("non-empty replay cannot follow terminal block anchor");
            };
            Some(AppliedExecBlockRange::new(
                first_block_num,
                first,
                last,
                blobs.len(),
            ))
        }
        _ => None,
    };

    Ok(ReplaySummary::new(
        applied,
        applied_evm_headers,
        applied_state_roots,
        final_state_root,
        state,
    ))
}

fn apply_da_blob(state: &mut EthereumState, blob: &DaBlob) -> Result<(), ReconstructError> {
    apply_batch_state_diff_to_ethereum_state(state, &blob.state_diff)
}

#[cfg(test)]
mod tests {
    use std::iter;

    use alloy_primitives::{Address, B256};
    use alpen_ee_da_types::{DaBlob, EvmHeaderSummary};
    use alpen_reth_statediff::{
        apply_batch_state_diff_to_ethereum_state, ethereum_state_from_genesis_accounts,
        test_utils::{
            addr, hash, slot, snapshot as account_snapshot, value as storage_or_balance_value,
        },
        AccountChange, AccountDiff, BatchStateDiff, EthereumStateExt, GenesisAccount, StorageDiff,
    };
    use proptest::prelude::*;
    use rsp_mpt::EthereumState;
    use strata_identifiers::Buf32;

    use crate::{
        replay_da_blobs_from_genesis, replay_da_blobs_from_snapshot, ReplayError,
        ReplayStateSnapshot, SNAPSHOT_FORMAT_VERSION,
    };

    const MAX_SEQUENCE_INCREMENT_SUM: u64 = 16;
    const MAX_SEQUENCE_LEN: usize = 8;

    #[test]
    fn test_empty_genesis_replay() {
        let state = build_empty_ethereum_state();

        let summary =
            replay_da_blobs_from_genesis(state.clone(), &[]).expect("replay must succeed");

        assert_eq!(summary.applied(), None);
        assert_eq!(summary.final_state_root(), state.state_root_buf32());
    }

    #[test]
    fn test_empty_snapshot_replay() {
        let snapshot = build_replay_snapshot(10, 100);
        let expected_state_root = snapshot.expected_state_root();

        let summary = replay_da_blobs_from_snapshot(snapshot, &[]).expect("replay must succeed");

        assert_eq!(summary.applied(), None);
        assert_eq!(summary.final_state_root(), expected_state_root);
        assert_eq!(
            summary.final_ethereum_state().state_root_buf32(),
            expected_state_root
        );
    }

    #[test]
    fn test_snapshot_replay() {
        let snapshot = build_replay_snapshot(5, 99);
        let blobs = vec![build_da_blob(5, 100), build_da_blob(6, 101)];

        let summary = replay_da_blobs_from_snapshot(snapshot, &blobs).expect("replay must succeed");
        let applied = summary.applied().expect("applied range must be populated");

        assert_eq!(applied.count(), 2);
        assert_eq!(applied.first_update_seq_no(), 5);
        assert_eq!(applied.last_update_seq_no(), 6);
        assert_eq!(applied.first_block_num(), 100);
        assert_eq!(applied.last_block_num(), 101);
        assert_eq!(
            summary.applied_evm_headers(),
            blobs.iter().map(|blob| blob.evm_header).collect::<Vec<_>>()
        );
        assert_eq!(summary.applied_state_roots().len(), 2);
    }

    #[test]
    fn test_snapshot_applied_range() {
        let snapshot = build_replay_snapshot(5, 100);
        let blobs = vec![build_da_blob(5, 105)];

        let summary = replay_da_blobs_from_snapshot(snapshot, &blobs).expect("replay must succeed");
        let applied = summary.applied().expect("applied range must be populated");

        assert_eq!(applied.first_block_num(), 101);
        assert_eq!(applied.last_block_num(), 105);
    }

    #[test]
    fn test_genesis_applied_range() {
        let blobs = vec![build_da_blob(0, 3)];

        let summary = replay_da_blobs_from_genesis(build_empty_ethereum_state(), &blobs)
            .expect("replay succeeds");
        let applied = summary.applied().expect("applied range must be populated");

        assert_eq!(applied.first_block_num(), 1);
        assert_eq!(applied.last_block_num(), 3);
    }

    #[test]
    fn test_snapshot_root_mismatch() {
        let expected = Buf32::from([0x42; 32]);
        let snapshot = parse_snapshot_json(build_snapshot_json_with_expected_state_root(expected));

        let err =
            replay_da_blobs_from_snapshot(snapshot, &[]).expect_err("replay must reject snapshot");

        match err {
            ReplayError::SnapshotRootMismatch {
                expected_state_root,
                ..
            } => assert_eq!(expected_state_root, expected),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn test_unsupported_snapshot_version() {
        let snapshot = parse_snapshot_json(build_snapshot_json_with_version(999));

        let err =
            replay_da_blobs_from_snapshot(snapshot, &[]).expect_err("replay must reject snapshot");

        match err {
            ReplayError::UnsupportedSnapshotVersion {
                version,
                supported_version,
            } => {
                assert_eq!(version, 999);
                assert_eq!(supported_version, SNAPSHOT_FORMAT_VERSION);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn test_snapshot_update_seq_no_mismatch() {
        let snapshot = build_replay_snapshot(5, 99);
        let blobs = vec![build_da_blob(6, 100)];

        let err =
            replay_da_blobs_from_snapshot(snapshot, &blobs).expect_err("replay must reject anchor");

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
    fn test_snapshot_block_anchor_mismatch() {
        let snapshot = build_replay_snapshot(5, 100);
        let blobs = vec![build_da_blob(5, 100)];

        let err =
            replay_da_blobs_from_snapshot(snapshot, &blobs).expect_err("replay must reject anchor");

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
    fn test_snapshot_update_seq_no_gap() {
        let snapshot = build_replay_snapshot(5, 99);
        let blobs = vec![build_da_blob(5, 100), build_da_blob(7, 101)];

        let err =
            replay_da_blobs_from_snapshot(snapshot, &blobs).expect_err("replay must reject gap");

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
        fn test_contiguous_genesis_sequence(
            first_block_num in 1u64..=(u64::MAX - MAX_SEQUENCE_INCREMENT_SUM),
            count in 1usize..=MAX_SEQUENCE_LEN,
        ) {
            let blobs = build_contiguous_da_blobs(first_block_num, count);

            let summary = replay_da_blobs_from_genesis(build_empty_ethereum_state(), &blobs).expect("replay must succeed");
            let applied = summary.applied().expect("applied range must be populated");
            prop_assert_eq!(applied.count(), count);
            prop_assert_eq!(applied.first_update_seq_no(), 0);
            prop_assert_eq!(applied.last_update_seq_no(), count as u64 - 1);
            prop_assert_eq!(applied.first_block_num(), 1);
            prop_assert_eq!(applied.last_block_num(), first_block_num + count as u64 - 1);
        }

        #[test]
        fn test_non_genesis_start(
            first_update_seq_no in 1u64..=u64::MAX,
            first_block_num in any::<u64>(),
        ) {
            let blobs = vec![build_da_blob(first_update_seq_no, first_block_num)];

            let err = replay_da_blobs_from_genesis(build_empty_ethereum_state(), &blobs).expect_err("replay must fail");
            match err {
                ReplayError::NonGenesisStart { first_update_seq_no: got } => {
                    prop_assert_eq!(got, first_update_seq_no);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn test_update_seq_no_gap(
            first_block_num in 1u64..=(u64::MAX - 2),
        ) {
            let blobs = vec![
                build_da_blob(0, first_block_num),
                build_da_blob(2, first_block_num + 1),
            ];

            let err = replay_da_blobs_from_genesis(build_empty_ethereum_state(), &blobs).expect_err("replay must fail");
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
        fn test_duplicate_update_seq_no(
            first_block_num in 1u64..=(u64::MAX - 2),
        ) {
            let blobs = vec![
                build_da_blob(0, first_block_num),
                build_da_blob(0, first_block_num + 1),
            ];

            let err = replay_da_blobs_from_genesis(build_empty_ethereum_state(), &blobs).expect_err("replay must fail");
            match err {
                ReplayError::DuplicateUpdateSeqNo { blob_index, update_seq_no: got } => {
                    prop_assert_eq!(blob_index, 1);
                    prop_assert_eq!(got, 0);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn test_non_increasing_block_number(
            first_block_num in 1u64..=u64::MAX,
            non_increase in 0u64..=3,
        ) {
            let second_block_num = first_block_num.saturating_sub(non_increase);
            let blobs = vec![
                build_da_blob(0, first_block_num),
                build_da_blob(1, second_block_num),
            ];

            let err = replay_da_blobs_from_genesis(build_empty_ethereum_state(), &blobs).expect_err("replay must fail");
            match err {
                ReplayError::NonIncreasingBlockNumber { blob_index, .. } => {
                    prop_assert_eq!(blob_index, 1);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn test_terminal_update_seq_no(
            block_num in 1u64..=(u64::MAX - 1),
        ) {
            let snapshot = build_replay_snapshot(u64::MAX, block_num - 1);
            let blobs = vec![build_da_blob(u64::MAX, block_num), build_da_blob(0, block_num + 1)];

            let err = replay_da_blobs_from_snapshot(snapshot, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::TerminalUpdateSeqNo { blob_index, update_seq_no } => {
                    prop_assert_eq!(blob_index, 1);
                    prop_assert_eq!(update_seq_no, u64::MAX);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }
    }

    #[test]
    fn test_non_empty_state_diffs() {
        let address = addr(0x44);
        let code_hash = hash(0x55);
        let diff0 = build_account_creation_diff(address, code_hash);
        let diff1 = build_account_update_diff(address, code_hash);
        let blobs = vec![
            build_da_blob_with_diff(0, 1, diff0.clone()),
            build_da_blob_with_diff(1, 2, diff1.clone()),
        ];

        let genesis = build_empty_ethereum_state();
        let mut expected_state = genesis.clone();
        apply_batch_state_diff_to_ethereum_state(&mut expected_state, &diff0)
            .expect("first diff applies");
        let after_first_root = expected_state.state_root_buf32();
        apply_batch_state_diff_to_ethereum_state(&mut expected_state, &diff1)
            .expect("second diff applies");
        let expected_final_root = expected_state.state_root_buf32();

        let summary = replay_da_blobs_from_genesis(genesis, &blobs).expect("replay must succeed");

        assert_ne!(after_first_root, expected_final_root);
        assert_eq!(
            summary.applied_state_roots(),
            &[after_first_root, expected_final_root]
        );
        assert_eq!(summary.final_state_root(), expected_final_root);
    }

    fn build_da_blob(update_seq_no: u64, block_num: u64) -> DaBlob {
        build_da_blob_with_diff(update_seq_no, block_num, BatchStateDiff::default())
    }

    fn build_da_blob_with_diff(
        update_seq_no: u64,
        block_num: u64,
        state_diff: BatchStateDiff,
    ) -> DaBlob {
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
            state_diff,
        }
    }

    fn build_contiguous_da_blobs(first_block_num: u64, count: usize) -> Vec<DaBlob> {
        (0..count)
            .map(|idx| build_da_blob(idx as u64, first_block_num + idx as u64))
            .collect()
    }

    fn build_replay_snapshot(
        next_update_seq_no: u64,
        last_applied_block_num: u64,
    ) -> ReplayStateSnapshot {
        let state = build_empty_ethereum_state();
        ReplayStateSnapshot::new(
            next_update_seq_no,
            last_applied_block_num,
            state,
            Default::default(),
        )
    }

    fn build_empty_ethereum_state() -> EthereumState {
        ethereum_state_from_genesis_accounts(iter::empty::<(Address, GenesisAccount)>())
            .expect("empty genesis state builds")
    }

    fn build_account_creation_diff(address: Address, code_hash: B256) -> BatchStateDiff {
        let mut state_diff = BatchStateDiff::new();
        state_diff.accounts.insert(
            address,
            AccountChange::Created(AccountDiff::new_created(
                storage_or_balance_value(10),
                1,
                code_hash,
            )),
        );
        let mut storage_diff = StorageDiff::new();
        storage_diff.set_slot(slot(1), storage_or_balance_value(11));
        state_diff.storage.insert(address, storage_diff);
        state_diff
    }

    fn build_account_update_diff(address: Address, code_hash: B256) -> BatchStateDiff {
        let original = account_snapshot(10, 1, code_hash);
        let current = account_snapshot(15, 2, code_hash);
        let mut state_diff = BatchStateDiff::new();
        state_diff.accounts.insert(
            address,
            AccountChange::Updated(
                AccountDiff::from_account_snapshot(&current, Some(&original), address)
                    .expect("account changed"),
            ),
        );
        let mut storage_diff = StorageDiff::new();
        storage_diff.set_slot(slot(1), storage_or_balance_value(12));
        state_diff.storage.insert(address, storage_diff);
        state_diff
    }

    fn build_snapshot_json_with_expected_state_root(
        expected_state_root: Buf32,
    ) -> serde_json::Value {
        build_snapshot_json(SNAPSHOT_FORMAT_VERSION, expected_state_root)
    }

    fn build_snapshot_json_with_version(version: u32) -> serde_json::Value {
        let expected_state_root = build_empty_ethereum_state().state_root_buf32();
        build_snapshot_json(version, expected_state_root)
    }

    fn build_snapshot_json(version: u32, expected_state_root: Buf32) -> serde_json::Value {
        serde_json::json!({
            "version": version,
            "expected_state_root": expected_state_root,
            "next_update_seq_no": 5,
            "last_applied_block_num": 99,
            "ethereum_state": build_empty_ethereum_state(),
            "bytecodes": {},
        })
    }

    fn parse_snapshot_json(value: serde_json::Value) -> ReplayStateSnapshot {
        serde_json::from_value(value).expect("snapshot JSON deserializes")
    }
}
