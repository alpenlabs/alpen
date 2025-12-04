//! Error types for mempool operations.

use strata_acct_types::AccountId;
use strata_identifiers::OLTxId;

/// Result type for mempool operations.
pub type MempoolResult<T> = Result<T, MempoolError>;

/// Errors that can occur during mempool operations.
#[derive(Debug, thiserror::Error)]
pub enum MempoolError {
    /// Transaction with the given ID doesn't exist.
    #[error("Transaction {0} not found in mempool")]
    TransactionNotFound(OLTxId),

    /// Failed to parse transaction blob into OLTransaction.
    #[error("Failed to parse transaction blob: {0}")]
    ParseError(String),

    /// Transaction has invalid or missing required fields.
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),

    /// Transaction's min_slot is in the future.
    #[error("Transaction too early: min_slot={min_slot}, current_slot={current_slot}")]
    TooEarly { min_slot: u64, current_slot: u64 },

    /// Transaction's max_slot has passed (expired).
    #[error("Transaction expired: max_slot={max_slot}")]
    Expired { max_slot: u64 },

    /// Transaction sequence number is invalid.
    #[error("Invalid sequence number: {0}")]
    InvalidSequenceNumber(u64),

    /// Transaction sequence number is stale (already processed).
    #[error("Stale sequence number: expected={expected}, provided={provided}")]
    StaleSequenceNumber { expected: u64, provided: u64 },

    /// Transaction sequence number is too far in the future.
    #[error("Sequence number too high: expected={expected}, provided={provided}")]
    SequenceNumberTooHigh { expected: u64, provided: u64 },

    /// Transaction is too large.
    #[error("Transaction too large: size={size}, max={max}")]
    TransactionTooLarge { size: usize, max: usize },

    /// Account has too many pending transactions.
    #[error("Account {0:?} has too many pending transactions")]
    AccountLimitExceeded(AccountId),

    /// Mempool is full (count limit exceeded).
    #[error("Mempool full: count={count}, max={max}")]
    MempoolCountLimitExceeded { count: usize, max: usize },

    /// Mempool is full (size limit exceeded).
    #[error("Mempool full: size={size}, max={max}")]
    MempoolSizeLimitExceeded { size: usize, max: usize },

    /// Database error occurred during mempool operation.
    #[error("Database error: {0}")]
    DatabaseError(String),

    /// Internal error (should not happen).
    #[error("Internal error: {0}")]
    Internal(String),
}
