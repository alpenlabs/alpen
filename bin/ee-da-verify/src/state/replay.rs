//! Replays reassembled DA blobs into a state reconstructor.
//!
//! Applied range must cover a contiguous block sequence derived from the
//! reassembled blobs.

use alpen_chainspec::chain_value_parser;
use alpen_ee_common::DaBlob;
use alpen_reth_statediff::{ReconstructError, StateReconstructor};
use serde::Serialize;
use strata_identifiers::{Buf32, ExecBlockCommitment};
use thiserror::Error;

/// Range of EVM exec blocks applied during replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AppliedExecBlockRange {
    pub(crate) first: ExecBlockCommitment,
    pub(crate) last: ExecBlockCommitment,
    pub(crate) count: usize,
}

/// Replay-stage output summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplaySummary {
    pub(crate) applied: Option<AppliedExecBlockRange>,
    pub(crate) final_state_root: Buf32,
}

/// Errors raised while replaying reassembled blobs.
#[derive(Debug, Error)]
pub(crate) enum ReplayError {
    #[error("invalid chain spec '{chain_spec}': {message}")]
    InvalidChainSpec { chain_spec: String, message: String },

    #[error(
        "first blob anchor mismatch: expected genesis prev_block {expected_prev_block}, got {actual_prev_block}"
    )]
    FirstBlobAnchorMismatch {
        expected_prev_block: String,
        actual_prev_block: String,
    },

    #[error(
        "blob {blob_index} has non-increasing block number: previous={previous_block_num}, current={current_block_num}"
    )]
    NonIncreasingBlockNumber {
        blob_index: usize,
        previous_block_num: u64,
        current_block_num: u64,
    },

    #[error(
        "blob {blob_index} linkage mismatch: expected prev_block {expected_prev_block}, got {actual_prev_block}"
    )]
    BatchLinkageMismatch {
        blob_index: usize,
        expected_prev_block: String,
        actual_prev_block: String,
    },

    #[error("failed applying state diff for blob {blob_index}: {source}")]
    ApplyDiff {
        blob_index: usize,
        #[source]
        source: ReconstructError,
    },
}

/// Replays reassembled DA blobs into the in-memory state reconstructor.
pub(crate) fn replay_blobs(
    chain_spec: &str,
    blobs: &[DaBlob],
) -> Result<ReplaySummary, ReplayError> {
    let mut reconstructor = StateReconstructor::from_chain_spec(chain_spec).map_err(|error| {
        ReplayError::InvalidChainSpec {
            chain_spec: chain_spec.to_owned(),
            message: error.to_string(),
        }
    })?;

    if let Some(first_blob) = blobs.first() {
        let genesis_anchor = resolve_genesis_anchor(chain_spec).map_err(|message| {
            ReplayError::InvalidChainSpec {
                chain_spec: chain_spec.to_owned(),
                message,
            }
        })?;
        let first_prev_block: [u8; 32] = first_blob.batch_id.prev_block().into();
        if first_prev_block != genesis_anchor {
            return Err(ReplayError::FirstBlobAnchorMismatch {
                expected_prev_block: bytes_to_hex(genesis_anchor),
                actual_prev_block: bytes_to_hex(first_prev_block),
            });
        }
    }

    let mut previous_block_num = None;
    let mut previous_last_block = None;

    for (blob_index, blob) in blobs.iter().enumerate() {
        let current_block_num = blob.evm_header.block_num;
        let current_prev_block: [u8; 32] = blob.batch_id.prev_block().into();

        if let Some(prev_block_num) = previous_block_num {
            if current_block_num <= prev_block_num {
                return Err(ReplayError::NonIncreasingBlockNumber {
                    blob_index,
                    previous_block_num: prev_block_num,
                    current_block_num,
                });
            }
        }

        if let Some(expected_prev_block) = previous_last_block {
            if current_prev_block != expected_prev_block {
                return Err(ReplayError::BatchLinkageMismatch {
                    blob_index,
                    expected_prev_block: bytes_to_hex(expected_prev_block),
                    actual_prev_block: bytes_to_hex(current_prev_block),
                });
            }
        }

        reconstructor
            .apply_diff(&blob.state_diff)
            .map_err(|source| ReplayError::ApplyDiff { blob_index, source })?;

        previous_block_num = Some(current_block_num);
        previous_last_block = Some(blob.batch_id.last_block().into());
    }

    let state_root_bytes: [u8; 32] = reconstructor.state_root().into();
    let final_state_root = Buf32::from(state_root_bytes);
    let applied = match (blobs.first(), blobs.last()) {
        (Some(first), Some(last)) => Some(AppliedExecBlockRange {
            first: blob_commitment(first),
            last: blob_commitment(last),
            count: blobs.len(),
        }),
        _ => None,
    };
    Ok(ReplaySummary {
        applied,
        final_state_root,
    })
}

fn blob_commitment(blob: &DaBlob) -> ExecBlockCommitment {
    let last_block: [u8; 32] = blob.batch_id.last_block().into();
    ExecBlockCommitment::new(blob.evm_header.block_num, Buf32::from(last_block))
}

fn resolve_genesis_anchor(chain_spec: &str) -> Result<[u8; 32], String> {
    let chain_spec = chain_value_parser(chain_spec).map_err(|error| error.to_string())?;
    Ok(chain_spec.genesis_hash().into())
}

fn bytes_to_hex(bytes: [u8; 32]) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use proptest::{collection, prelude::*};

    use super::{replay_blobs, ReplayError};
    use crate::state::test_utils::{
        hash_from_seed, make_block_numbers, make_linked_blobs, test_blob, MAX_LINKAGE_STEP,
        MAX_SEQUENCE_INCREMENT, MAX_SEQUENCE_INCREMENT_SUM, MAX_SEQUENCE_LEN, TEST_CHAIN_SPEC,
    };

    #[test]
    fn replay_blobs_returns_empty_summary_for_empty_input() {
        let summary = replay_blobs(TEST_CHAIN_SPEC, &[]).expect("replay must succeed");
        assert_eq!(summary.applied, None);
    }

    proptest! {
        #[test]
        fn replay_blobs_applies_valid_linked_sequence(
            start in 0u64..=(u64::MAX - MAX_SEQUENCE_INCREMENT_SUM),
            increments in collection::vec(1u64..=MAX_SEQUENCE_INCREMENT, 1..=MAX_SEQUENCE_LEN),
        ) {
            let block_numbers = make_block_numbers(start, &increments);
            let genesis_anchor =
                super::resolve_genesis_anchor(TEST_CHAIN_SPEC).expect("chain spec must parse");
            let blobs = make_linked_blobs(genesis_anchor, &block_numbers);

            let summary = replay_blobs(TEST_CHAIN_SPEC, &blobs).expect("replay must succeed");
            let applied = summary.applied.expect("applied range must be populated");
            prop_assert_eq!(applied.count, block_numbers.len());
            prop_assert_eq!(Some(applied.first.slot()), block_numbers.first().copied());
            prop_assert_eq!(Some(applied.last.slot()), block_numbers.last().copied());
        }

        #[test]
        fn replay_blobs_rejects_non_increasing_block_number(
            first_block_num in any::<u64>(),
            non_increase in 0u64..=3,
        ) {
            let genesis_anchor =
                super::resolve_genesis_anchor(TEST_CHAIN_SPEC).expect("chain spec must parse");
            let first_last = hash_from_seed(1);
            let second_last = hash_from_seed(2);
            let second_block_num = first_block_num.saturating_sub(non_increase);

            let blobs = vec![
                test_blob(genesis_anchor, first_last, first_block_num),
                test_blob(first_last, second_last, second_block_num),
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
        fn replay_blobs_rejects_batch_linkage_mismatch(
            first_block_num in 0u64..=(u64::MAX - MAX_LINKAGE_STEP),
            step in 1u64..=MAX_LINKAGE_STEP,
            wrong_prev in any::<[u8; 32]>(),
        ) {
            let genesis_anchor =
                super::resolve_genesis_anchor(TEST_CHAIN_SPEC).expect("chain spec must parse");
            let first_last = hash_from_seed(1);
            let second_last = hash_from_seed(2);
            prop_assume!(wrong_prev != first_last);

            let blobs = vec![
                test_blob(genesis_anchor, first_last, first_block_num),
                test_blob(wrong_prev, second_last, first_block_num + step),
            ];

            let err = replay_blobs(TEST_CHAIN_SPEC, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::BatchLinkageMismatch { blob_index, .. } => {
                    prop_assert_eq!(blob_index, 1);
                }
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        #[test]
        fn replay_blobs_rejects_first_blob_anchor_mismatch(
            block_num in any::<u64>(),
            wrong_anchor in any::<[u8; 32]>(),
        ) {
            let genesis_anchor =
                super::resolve_genesis_anchor(TEST_CHAIN_SPEC).expect("chain spec must parse");
            prop_assume!(wrong_anchor != genesis_anchor);

            let blobs = vec![test_blob(wrong_anchor, hash_from_seed(0), block_num)];
            let err = replay_blobs(TEST_CHAIN_SPEC, &blobs).expect_err("replay must fail");
            match err {
                ReplayError::FirstBlobAnchorMismatch { .. } => {}
                other => prop_assert!(false, "unexpected error: {other}"),
            }
        }
    }
}
