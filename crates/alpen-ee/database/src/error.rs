use alpen_ee_common::StorageError;
use sled::transaction::TransactionError;
use strata_identifiers::OLBlockId;
use strata_storage_common::exec::OpsError;
use thiserror::Error;
use typed_sled::error::Error as SledError;

pub type DbResult<T> = Result<T, DbError>;

/// Database-specific errors.
#[derive(Debug, Clone, Error)]
pub enum DbError {
    /// Attempted to persist a null OL block.
    #[error("null Ol block should not be persisted")]
    NullOlBlock,

    /// OL slot was skipped in sequential persistence.
    #[error("OL entries must be persisted sequentially; next: {expected}; got: {got}")]
    SkippedOlSlot { expected: u64, got: u64 },

    /// Transaction conflict: slot is already filled.
    #[error("Txn conflict: OL slot {0} already filled")]
    TxnFilledOlSlot(u64),

    /// Transaction conflict: expected slot to be empty.
    #[error("Txn conflict: OL slot {0} should be empty")]
    TxnExpectEmptyOlSlot(u64),

    /// Account state is missing for the given block.
    #[error("Account state expected to be present; block_id = {0}")]
    MissingAccountState(OLBlockId),

    /// Database operation error.
    #[error("Database: {0}")]
    DbOpsError(#[from] OpsError),

    /// Sled database error.
    #[error("sled: {0}")]
    Sled(String),

    /// Sled transaction error.
    #[error("sled txn: {0}")]
    SledTxn(String),

    /// Other unspecified database error.
    #[error("{0}")]
    #[allow(dead_code, clippy::allow_attributes, reason = "feature gated")]
    Other(String),
}

impl DbError {
    pub(crate) fn skipped_ol_slot(expected: u64, got: u64) -> DbError {
        DbError::SkippedOlSlot { expected, got }
    }
}

impl From<SledError> for DbError {
    fn from(maybe_dberr: SledError) -> Self {
        match maybe_dberr.downcast_abort::<DbError>() {
            Ok(dberr) => dberr,
            Err(other) => DbError::Sled(other.to_string()),
        }
    }
}

impl From<TransactionError<SledError>> for DbError {
    fn from(value: TransactionError<SledError>) -> Self {
        match value {
            TransactionError::Abort(tsled_err) => tsled_err.into(),
            err => DbError::SledTxn(err.to_string()),
        }
    }
}

impl From<DbError> for StorageError {
    fn from(err: DbError) -> Self {
        match err {
            DbError::SkippedOlSlot { expected, got } => StorageError::MissingSlot {
                attempted_slot: got,
                last_slot: expected,
            },
            e => StorageError::database(e.to_string()),
        }
    }
}
