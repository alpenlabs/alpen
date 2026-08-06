//! Replay error types.

use alpen_reth_statediff::ReconstructError;
use strata_identifiers::Buf32;

/// Errors raised while replaying decoded DA blobs.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    /// Genesis replay starts after the first EE DA update.
    #[error("non-genesis start: expected update_seq_no 0, got {first_update_seq_no}")]
    NonGenesisStart { first_update_seq_no: u64 },

    /// The supplied snapshot artifact version is not supported.
    #[error("unsupported snapshot version: expected {supported_version}, got {version}")]
    UnsupportedSnapshotVersion {
        version: u32,
        supported_version: u32,
    },

    /// The supplied snapshot state does not produce its claimed state root.
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
    #[error("snapshot block anchor mismatch: expected first block after {last_applied_block_num}, got {first_blob_block_num}")]
    SnapshotBlockAnchorMismatch {
        last_applied_block_num: u64,
        first_blob_block_num: u64,
    },

    /// The DA blob sequence skips an update sequence number.
    #[error("update_seq_no gap at blob {blob_index}: expected {expected_update_seq_no}, got {actual_update_seq_no}")]
    UpdateSeqNoGap {
        blob_index: usize,
        expected_update_seq_no: u64,
        actual_update_seq_no: u64,
    },

    /// The DA blob sequence cannot continue after the terminal update sequence number.
    #[error("terminal update_seq_no {update_seq_no} at blob {blob_index}")]
    TerminalUpdateSeqNo {
        blob_index: usize,
        update_seq_no: u64,
    },

    /// The DA blob sequence repeats an update sequence number.
    #[error("duplicate update_seq_no {update_seq_no} at blob {blob_index}")]
    DuplicateUpdateSeqNo {
        blob_index: usize,
        update_seq_no: u64,
    },

    /// The DA blob sequence does not strictly increase EVM block numbers.
    #[error("non-increasing block number at blob {blob_index}: expected greater than {previous_block_num}, got {current_block_num}")]
    NonIncreasingBlockNumber {
        blob_index: usize,
        previous_block_num: u64,
        current_block_num: u64,
    },

    /// The Ethereum state replay path rejects a blob state diff.
    #[error("failed to apply state diff for blob {blob_index}: {source}")]
    ApplyDiff {
        blob_index: usize,
        #[source]
        source: ReconstructError,
    },
}
