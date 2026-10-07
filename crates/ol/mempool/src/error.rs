//! OL mempool error types.

use std::convert::Infallible;

use strata_acct_types::AccountId;
use strata_db_types::DbError;
use strata_identifiers::{EpochCommitment, OLTxId};
use strata_ol_log_budget::TxLogBudgetError;
use strata_ol_stf::{EpochSpecSelectionError, InvalidEpochL1Range, UpgradeRequired};

/// Errors that can occur during mempool operations.
#[derive(Debug, thiserror::Error)]
pub enum OLMempoolError {
    /// A transaction exceeds its standalone log budget, or its logs cannot be encoded.
    #[error(transparent)]
    LogBudget(#[from] TxLogBudgetError),

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

    /// State provider error.
    #[error("state provider: {0}")]
    StateProvider(String),

    /// Serialization/deserialization error.
    #[error("serialization: {0}")]
    Serialization(String),

    /// Mempool service is closed or unavailable.
    #[error("mempool service unavailable: {0}")]
    ServiceClosed(String),

    /// Mempool is not running on this node (e.g. checkpoint-sync fullnode).
    #[error("mempool not available on this node")]
    NotAvailable,

    /// The block after the tip runs a spec this binary does not implement, so
    /// the mempool admits no transaction.
    #[error(transparent)]
    UpgradeRequired(#[from] UpgradeRequired),

    /// The tip ends an epoch whose summary, or whose predecessor's summary, is
    /// not stored.
    #[error("missing the epoch summary of {0}")]
    MissingEpochSummary(EpochCommitment),

    /// The tip epoch's last L1 block does not follow the previous epoch's.
    #[error("epoch L1 range: {0}")]
    InvalidEpochL1Range(#[from] InvalidEpochL1Range),

    /// Selecting the spec of the block after the tip failed.
    #[error("select the spec of the block after the tip: {0}")]
    SpecSelection(#[source] EpochSpecSelectionError<Infallible>),
}
