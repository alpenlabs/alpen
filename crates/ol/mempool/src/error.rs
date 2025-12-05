//! OL mempool error types.

use strata_db_types::DbError;
use strata_identifiers::OLTxId;

/// Errors that can occur during mempool operations.
#[derive(Debug, thiserror::Error)]
pub enum OLMempoolError {
    /// Transaction with the given ID doesn't exist.
    #[error("transaction {0} not found in mempool")]
    TransactionNotFound(OLTxId),

    /// Mempool is full (capacity limit reached).
    #[error("mempool is full: current={current}, limit={limit}")]
    MempoolFull { current: usize, limit: usize },

    /// Transaction size exceeds limit.
    #[error("transaction size {size} bytes exceeds limit {limit} bytes")]
    TransactionTooLarge { size: usize, limit: usize },

    /// Transaction has expired (max_slot is in the past).
    #[error("transaction {txid} has expired: max_slot={max_slot}, current_slot={current_slot}")]
    TransactionExpired {
        txid: OLTxId,
        max_slot: u64,
        current_slot: u64,
    },

    /// Transaction is not yet valid (min_slot is in the future).
    #[error(
        "transaction {txid} is not yet valid: min_slot={min_slot}, current_slot={current_slot}"
    )]
    TransactionNotYetValid {
        txid: OLTxId,
        min_slot: u64,
        current_slot: u64,
    },

    /// Database error.
    #[error("database error: {0}")]
    Database(#[from] DbError),

    /// Serialization/deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),
}
