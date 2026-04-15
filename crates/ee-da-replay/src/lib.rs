//! Replays decoded EE DA blobs into reconstructed execution state.

use alpen_chainspec::chain_value_parser;
use alpen_ee_common::DaBlob;
use alpen_reth_statediff::{ReconstructError, StateReconstructor};
use serde::Serialize;
use strata_identifiers::Buf32;
use thiserror::Error;

/// Range of EVM execution blocks applied during replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppliedExecBlockRange {
    first_block_num: u64,
    first_block_hash: Buf32,
    last_block_num: u64,
    last_block_hash: Buf32,
    count: usize,
}

impl AppliedExecBlockRange {
    fn new(first: &DaBlob, last: &DaBlob, count: usize) -> Self {
        Self {
            first_block_num: first.evm_header.block_num,
            first_block_hash: Buf32::from(blob_last_block_hash(first)),
            last_block_num: last.evm_header.block_num,
            last_block_hash: Buf32::from(blob_last_block_hash(last)),
            count,
        }
    }

    /// Returns the first applied EVM block number.
    pub fn first_block_num(&self) -> u64 {
        self.first_block_num
    }

    /// Returns the first applied EVM block hash.
    pub fn first_block_hash(&self) -> Buf32 {
        self.first_block_hash
    }

    /// Returns the last applied EVM block number.
    pub fn last_block_num(&self) -> u64 {
        self.last_block_num
    }

    /// Returns the last applied EVM block hash.
    pub fn last_block_hash(&self) -> Buf32 {
        self.last_block_hash
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

/// Replays ordered DA blobs into the in-memory state reconstructor.
pub fn replay_blobs(chain_spec: &str, blobs: &[DaBlob]) -> Result<ReplaySummary, ReplayError> {
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
        let first_prev_block = blob_prev_block_hash(first_blob);
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
        let current_prev_block = blob_prev_block_hash(blob);

        if let Some(previous_block_num) = previous_block_num {
            if current_block_num <= previous_block_num {
                return Err(ReplayError::NonIncreasingBlockNumber {
                    blob_index,
                    previous_block_num,
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
        previous_last_block = Some(blob_last_block_hash(blob));
    }

    let state_root_bytes: [u8; 32] = reconstructor.state_root().into();
    let final_state_root = Buf32::from(state_root_bytes);
    let applied = match (blobs.first(), blobs.last()) {
        (Some(first), Some(last)) => Some(AppliedExecBlockRange::new(first, last, blobs.len())),
        _ => None,
    };

    Ok(ReplaySummary::new(applied, final_state_root))
}

fn resolve_genesis_anchor(chain_spec: &str) -> Result<[u8; 32], String> {
    let chain_spec = chain_value_parser(chain_spec).map_err(|error| error.to_string())?;
    Ok(chain_spec.genesis_hash().into())
}

fn blob_prev_block_hash(blob: &DaBlob) -> [u8; 32] {
    blob.batch_id.prev_block().into()
}

fn blob_last_block_hash(blob: &DaBlob) -> [u8; 32] {
    blob.batch_id.last_block().into()
}

fn bytes_to_hex(bytes: [u8; 32]) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{BatchId, DaBlob, EvmHeaderSummary};
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
        fn replay_blobs_applies_valid_linked_sequence(
            start in 0u64..=(u64::MAX - MAX_SEQUENCE_INCREMENT_SUM),
            increments in collection::vec(1u64..=MAX_SEQUENCE_INCREMENT, 1..=MAX_SEQUENCE_LEN),
        ) {
            let block_numbers = make_block_numbers(start, &increments);
            let genesis_anchor =
                super::resolve_genesis_anchor(TEST_CHAIN_SPEC).expect("chain spec must parse");
            let blobs = make_linked_blobs(genesis_anchor, &block_numbers);

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

    fn test_blob(prev_block: [u8; 32], last_block: [u8; 32], block_num: u64) -> DaBlob {
        let gas_used = block_num % 1_000;
        DaBlob {
            batch_id: BatchId::from_parts(prev_block.into(), last_block.into()),
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

    fn hash_from_seed(seed: u64) -> [u8; 32] {
        let mut hash = [0u8; 32];
        hash[24..].copy_from_slice(&seed.to_be_bytes());
        hash
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

    fn make_linked_blobs(genesis_anchor: [u8; 32], block_numbers: &[u64]) -> Vec<DaBlob> {
        let mut blobs = Vec::with_capacity(block_numbers.len());
        let mut prev_block = genesis_anchor;

        for (index, block_num) in block_numbers.iter().enumerate() {
            let last_block = hash_from_seed(index as u64);
            blobs.push(test_blob(prev_block, last_block, *block_num));
            prev_block = last_block;
        }

        blobs
    }
}
