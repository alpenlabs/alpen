//! Replays decoded EE DA blobs into reconstructed execution state.

use alpen_ee_common::DaBlob;
use alpen_reth_statediff::{ReconstructError, StateReconstructor};
use serde::Serialize;
use strata_identifiers::Buf32;
use thiserror::Error;

/// Range of EVM execution blocks applied during replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppliedExecBlockRange {
    first_block_num: u64,
    last_block_num: u64,
    count: usize,
}

impl AppliedExecBlockRange {
    fn new(first: &DaBlob, last: &DaBlob, count: usize) -> Self {
        Self {
            first_block_num: first.evm_header.block_num,
            last_block_num: last.evm_header.block_num,
            count,
        }
    }

    /// Returns the first applied EVM block number.
    pub fn first_block_num(&self) -> u64 {
        self.first_block_num
    }

    /// Returns the last applied EVM block number.
    pub fn last_block_num(&self) -> u64 {
        self.last_block_num
    }

    /// Returns the number of DA blobs applied.
    pub fn count(&self) -> usize {
        self.count
    }
}

/// Replay-stage output summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplaySummary {
    applied: Option<AppliedExecBlockRange>,
    final_state_root: Buf32,
}

impl ReplaySummary {
    fn new(applied: Option<AppliedExecBlockRange>, final_state_root: Buf32) -> Self {
        Self {
            applied,
            final_state_root,
        }
    }

    /// Returns the applied EVM block range, if any DA blobs were replayed.
    pub fn applied(&self) -> Option<&AppliedExecBlockRange> {
        self.applied.as_ref()
    }

    /// Returns the reconstructed final state root.
    pub fn final_state_root(&self) -> Buf32 {
        self.final_state_root
    }
}

/// Errors raised while replaying decoded DA blobs.
#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("invalid chain spec '{chain_spec}': {message}")]
    InvalidChainSpec { chain_spec: String, message: String },

    #[error("first blob has update_seq_no {actual}, expected {expected}")]
    FirstUpdateSeqNoMismatch { expected: u64, actual: u64 },

    #[error("blob {blob_index} has update_seq_no {actual}, expected {expected}")]
    NonConsecutiveUpdateSeqNo {
        blob_index: usize,
        expected: u64,
        actual: u64,
    },

    #[error(
        "blob {blob_index} has non-increasing block number: previous={previous_block_num}, current={current_block_num}"
    )]
    NonIncreasingBlockNumber {
        blob_index: usize,
        previous_block_num: u64,
        current_block_num: u64,
    },

    #[error("failed applying state diff for blob {blob_index}: {source}")]
    ApplyDiff {
        blob_index: usize,
        #[source]
        source: ReconstructError,
    },
}

/// Replays ordered DA blobs into the in-memory state reconstructor.
pub fn replay_blobs(chain_spec: &str, blobs: &[DaBlob]) -> Result<ReplaySummary, ReplayError> {
    let mut reconstructor = StateReconstructor::from_chain_spec(chain_spec).map_err(|error| {
        ReplayError::InvalidChainSpec {
            chain_spec: chain_spec.to_owned(),
            message: error.to_string(),
        }
    })?;

    if let Some(first_blob) = blobs.first() {
        if first_blob.update_seq_no != 0 {
            return Err(ReplayError::FirstUpdateSeqNoMismatch {
                expected: 0,
                actual: first_blob.update_seq_no,
            });
        }
    }

    let mut expected_update_seq_no = 0u64;
    let mut previous_block_num = None;

    for (blob_index, blob) in blobs.iter().enumerate() {
        if blob.update_seq_no != expected_update_seq_no {
            return Err(ReplayError::NonConsecutiveUpdateSeqNo {
                blob_index,
                expected: expected_update_seq_no,
                actual: blob.update_seq_no,
            });
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

        expected_update_seq_no = expected_update_seq_no
            .checked_add(1)
            .expect("number of DA blobs fits in u64");
        previous_block_num = Some(current_block_num);
    }

    let state_root_bytes: [u8; 32] = reconstructor.state_root().into();
    let final_state_root = Buf32::from(state_root_bytes);
    let applied = match (blobs.first(), blobs.last()) {
        (Some(first), Some(last)) => Some(AppliedExecBlockRange::new(first, last, blobs.len())),
        _ => None,
    };

    Ok(ReplaySummary::new(applied, final_state_root))
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{DaBlob, EvmHeaderSummary};
    use proptest::{collection, prelude::*};

    use super::{replay_blobs, ReplayError};

    const TEST_CHAIN_SPEC: &str = "dev";
    const MAX_SEQUENCE_INCREMENT: u64 = 10;
    const MAX_SEQUENCE_LEN: usize = 8;
    const MAX_SEQUENCE_INCREMENT_SUM: u64 = MAX_SEQUENCE_INCREMENT * MAX_SEQUENCE_LEN as u64;
    const MAX_LINKAGE_STEP: u64 = 5;

    #[test]
    fn replay_blobs_returns_empty_summary_for_empty_input() {
        let summary = replay_blobs(TEST_CHAIN_SPEC, &[]).expect("replay must succeed");
        assert_eq!(summary.applied(), None);
    }

    proptest! {
        #[test]
        fn replay_blobs_applies_valid_sequence(
            start in 0u64..=(u64::MAX - MAX_SEQUENCE_INCREMENT_SUM),
            increments in collection::vec(1u64..=MAX_SEQUENCE_INCREMENT, 1..=MAX_SEQUENCE_LEN),
        ) {
            let block_numbers = make_block_numbers(start, &increments);
            let blobs = block_numbers
                .iter()
                .copied()
                .enumerate()
                .map(|(index, block_num)| test_blob(index as u64, block_num))
                .collect::<Vec<_>>();

            let summary = replay_blobs(TEST_CHAIN_SPEC, &blobs).expect("replay must succeed");
            let applied = summary.applied().expect("applied range must be populated");
            prop_assert_eq!(applied.count(), block_numbers.len());
            prop_assert_eq!(Some(applied.first_block_num()), block_numbers.first().copied());
            prop_assert_eq!(Some(applied.last_block_num()), block_numbers.last().copied());
        }

        #[test]
        fn replay_blobs_rejects_non_increasing_block_number(
            first_block_num in any::<u64>(),
            non_increase in 0u64..=3,
        ) {
            let second_block_num = first_block_num.saturating_sub(non_increase);
            let blobs = vec![
                test_blob(0, first_block_num),
                test_blob(1, second_block_num),
            ];

            let err = replay_blobs(TEST_CHAIN_SPEC, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::NonIncreasingBlockNumber { blob_index, .. } => {
                    prop_assert_eq!(blob_index, 1);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn replay_blobs_rejects_non_consecutive_update_seq_no(
            first_block_num in 0u64..=(u64::MAX - MAX_LINKAGE_STEP),
            step in 1u64..=MAX_LINKAGE_STEP,
            bad_seq_no in prop_oneof![Just(0u64), 2u64..=10],
        ) {
            let blobs = vec![
                test_blob(0, first_block_num),
                test_blob(bad_seq_no, first_block_num + step),
            ];

            let err = replay_blobs(TEST_CHAIN_SPEC, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::NonConsecutiveUpdateSeqNo { blob_index, .. } => {
                    prop_assert_eq!(blob_index, 1);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn replay_blobs_rejects_first_update_seq_no_mismatch(
            block_num in any::<u64>(),
            first_seq_no in 1u64..=10,
        ) {
            let blobs = vec![test_blob(first_seq_no, block_num)];
            let err = replay_blobs(TEST_CHAIN_SPEC, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::FirstUpdateSeqNoMismatch { expected, actual } => {
                    prop_assert_eq!(expected, 0);
                    prop_assert_eq!(actual, first_seq_no);
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

    fn make_block_numbers(start: u64, increments: &[u64]) -> Vec<u64> {
        let mut block_numbers = Vec::with_capacity(increments.len());
        let mut current = start;
        for increment in increments {
            current = current.saturating_add(*increment);
            block_numbers.push(current);
        }
        block_numbers
    }
}
