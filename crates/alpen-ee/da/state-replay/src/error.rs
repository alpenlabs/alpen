//! Replay error types.

use alpen_reth_statediff::ReconstructError;
use strata_identifiers::Buf32;

/// Errors raised while replaying decoded DA blobs.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    /// Genesis replay starts after the first EE DA update.
    #[error(
        "genesis replay requires first update_seq_no 0; first blob has update_seq_no {first_update_seq_no}"
    )]
    NonGenesisStart { first_update_seq_no: u64 },

    /// The supplied snapshot prestate does not produce its claimed state root.
    #[error("snapshot root mismatch: expected {expected_state_root}, got {actual_state_root}")]
    SnapshotRootMismatch {
        expected_state_root: Buf32,
        actual_state_root: Buf32,
    },

    /// The first replayed blob does not match the snapshot update anchor.
    #[error("snapshot update_seq_no mismatch: expected {expected_update_seq_no}, got {actual_update_seq_no}")]
    SnapshotUpdateSeqNoMismatch {
        expected_update_seq_no: u64,
        actual_update_seq_no: u64,
    },

    /// The first replayed blob does not follow the snapshot block anchor.
    #[error("snapshot block anchor mismatch: expected first block > {last_applied_block_num}, got {first_blob_block_num}")]
    SnapshotBlockAnchorMismatch {
        last_applied_block_num: u64,
        first_blob_block_num: u64,
    },

    /// The reconstructor fails to initialize from the snapshot prestate.
    #[error("invalid snapshot state: {source}")]
    InvalidSnapshotState {
        #[source]
        source: ReconstructError,
    },

    /// The DA blob sequence skips an update sequence number.
    #[error(
        "blob {blob_index} has update_seq_no gap: expected {expected_update_seq_no}, got {actual_update_seq_no}"
    )]
    UpdateSeqNoGap {
        blob_index: usize,
        expected_update_seq_no: u64,
        actual_update_seq_no: u64,
    },

    /// The DA blob sequence repeats an update sequence number.
    #[error("blob {blob_index} has duplicate update_seq_no {update_seq_no}")]
    DuplicateUpdateSeqNo {
        blob_index: usize,
        update_seq_no: u64,
    },

    /// The DA blob sequence does not strictly increase EVM block numbers.
    #[error(
        "blob {blob_index} has non-increasing block number: previous={previous_block_num}, current={current_block_num}"
    )]
    NonIncreasingBlockNumber {
        blob_index: usize,
        previous_block_num: u64,
        current_block_num: u64,
    },

    /// The state reconstructor rejects a blob state diff.
    #[error("failed to apply state diff for blob {blob_index}: {source}")]
    ApplyDiff {
        blob_index: usize,
        #[source]
        source: ReconstructError,
    },
}
