//! OL mempool error types.

use strata_acct_types::AccountId;
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

    /// Target account does not exist.
    #[error("account {account} does not exist")]
    AccountDoesNotExist { account: AccountId },

    /// Transaction targets wrong account type.
    #[error("transaction {txid} targets account {account} with incorrect type")]
    AccountTypeMismatch { txid: OLTxId, account: AccountId },

    /// Transaction sequence number is invalid (less than account's current sequence number).
    #[error(
        "transaction {txid} has invalid sequence number {tx_seq_no}, account current sequence number is {account_seq_no}"
    )]
    InvalidSequenceNumber {
        txid: OLTxId,
        tx_seq_no: u64,
        account_seq_no: u64,
    },

    /// Sequence number gap detected (expected sequential order).
    #[error("sequence number gap: expected {expected}, got {actual}")]
    SequenceNumberGap { expected: u64, actual: u64 },

    /// Account state access error (from StateAccessor).
    #[error("account state access error: {0}")]
    AccountStateAccess(String),

    /// Database error.
    #[error("database error: {0}")]
    Database(#[from] DbError),

    /// Serialization/deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Mempool service is closed or unavailable.
    #[error("mempool service unavailable: {0}")]
    ServiceClosed(String),
}
