//! OL mempool error types.

use strata_acct_types::AccountId;
use strata_db_types::DbError;
use strata_identifiers::OLTxId;

/// Errors that can occur during mempool operations.
#[derive(Debug, thiserror::Error)]
pub enum OLMempoolError {
    /// Mempool is full (transaction count limit reached).
    #[error("mempool is full: current={current}, limit={limit}")]
    MempoolFull { current: usize, limit: usize },

    /// Mempool byte size limit exceeded.
    #[error("mempool byte size limit exceeded: current={current}, limit={limit}")]
    MempoolByteLimitExceeded { current: usize, limit: usize },

    /// Account state access error (from StateAccessor).
    #[error("account state access: {0}")]
    AccountStateAccess(String),

    /// Target account does not exist.
    #[error("account {account} does not exist")]
    AccountDoesNotExist { account: AccountId },

    /// Transaction targets wrong account type.
    #[error("transaction {txid} targets account {account} with incorrect type")]
    AccountTypeMismatch { txid: OLTxId, account: AccountId },

    /// Transaction with the given ID doesn't exist.
    #[error("transaction {0} not found in mempool")]
    TransactionNotFound(OLTxId),

    /// Transaction size exceeds limit.
    #[error("transaction size {size} bytes exceeds limit {limit} bytes")]
    TransactionTooLarge { size: usize, limit: usize },

    /// Transaction has expired (max_slot has passed).
    #[error("transaction {txid} has expired: max_slot={max_slot}, current_slot={current_slot}")]
    TransactionExpired {
        txid: OLTxId,
        max_slot: u64,
        current_slot: u64,
    },

    /// Transaction is not mature (min_slot has not arrived).
    #[error("transaction {txid} is not mature: min_slot={min_slot}, current_slot={current_slot}")]
    TransactionNotMature {
        txid: OLTxId,
        min_slot: u64,
        current_slot: u64,
    },

    /// Transaction sequence number is already used.
    #[error(
        "transaction {txid} has already used sequencer number: expected={expected}, actual={actual}"
    )]
    UsedSequenceNumber {
        txid: OLTxId,
        expected: u64,
        actual: u64,
    },

    /// Sequence number gap detected (expected sequential order).
    #[error("sequence number gap: expected {expected}, actual {actual}")]
    SequenceNumberGap { expected: u64, actual: u64 },

    /// Database error.
    #[error("database: {0}")]
    Database(#[from] DbError),

    /// Serialization/deserialization error.
    #[error("serialization: {0}")]
    Serialization(String),

    /// Mempool service is closed or unavailable.
    #[error("mempool service unavailable: {0}")]
    ServiceClosed(String),
}
