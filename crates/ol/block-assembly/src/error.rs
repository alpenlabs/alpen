//! Error types for block assembly operations.

use strata_db_types::errors::DbError;
use strata_identifiers::{AccountId, Hash};
use strata_ol_mempool::OLMempoolError;

/// Errors that can occur during block assembly operations.
#[derive(Debug, thiserror::Error)]
pub enum BlockAssemblyError {
    /// Database operation failed.
    #[error("db: {0}")]
    Database(#[from] DbError),

    /// Mempool operation failed.
    #[error("mempool: {0}")]
    Mempool(#[from] OLMempoolError),

    /// Invalid L1 block range where `from_block` height > `to_block` height.
    #[error("invalid L1 block height range (from {from_height} to {to_height})")]
    InvalidRange { from_height: u64, to_height: u64 },

    /// L1 header claim hash does not match MMR entry.
    #[error("L1 header hash mismatch at index {idx}: expected {expected}, got {actual}")]
    L1HeaderHashMismatch {
        idx: u64,
        expected: Hash,
        actual: Hash,
    },

    /// L1 header claim references non-existent MMR leaf.
    #[error("L1 header leaf not found at index {0}")]
    L1HeaderLeafNotFound(u64),

    /// Inbox message leaf not found in MMR.
    #[error("inbox leaf not found at index {idx} for account {account_id}")]
    InboxLeafNotFound { idx: u64, account_id: AccountId },

    /// Inbox message hash does not match MMR entry.
    #[error(
        "inbox hash mismatch at index {idx} for account {account_id}: expected {expected}, got {actual}"
    )]
    InboxEntryHashMismatch {
        idx: u64,
        account_id: AccountId,
        expected: Hash,
        actual: Hash,
    },

    /// Invalid MMR range requested.
    #[error("invalid MMR range {start}..{end}")]
    InvalidMmrRange { start: u64, end: u64 },

    /// Account not found when validating transaction.
    #[error("account not found: {0}")]
    AccountNotFound(AccountId),

    /// Inbox MMR proof count mismatch.
    #[error("inbox MMR proof count mismatch (expected {expected}, got {got})")]
    InboxProofCountMismatch { expected: usize, got: usize },

    /// Other unexpected error.
    #[error("other: {0}")]
    Other(String),
}
