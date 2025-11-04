use strata_identifiers::OLBlockId;
use strata_storage_common::exec::OpsError;
use thiserror::Error;

use crate::traits::error::StorageError;

#[derive(Debug, Clone, Error)]
pub(crate) enum DbError {
    #[error("null Ol block should not be persisted")]
    NullOlBlock,

    #[error("OL entries must be persisted sequentially; next: {expected}; got: {got}")]
    SkippedOlSlot { expected: u64, got: u64 },

    #[error("Txn conflict: OL slot {0} already filled")]
    TxnFilledOlSlot(u64),

    #[error("Txn conflict: OL slot {0} should be empty")]
    #[expect(dead_code, reason = "wip")]
    TxnExpectEmptyOlSlot(u64),

    #[error("Account state expected to be present; block_id = {0}")]
    MissingAccountState(OLBlockId),

    // #[error("Account state expected to be present; slot = {0}")]
    // MissingAccountStateSlot(u64),
    #[error("Database: {0}")]
    DbOpsError(#[from] OpsError),

    #[cfg(feature = "sled")]
    #[error("sled: {0}")]
    Sled(String),

    #[cfg(feature = "sled")]
    #[error("sled txn: {0}")]
    SledTxn(String),

    #[cfg(feature = "rocksdb")]
    #[error("rocksdb txn: {0}")]
    RocksDBTxn(String),

    #[error("{0}")]
    #[allow(dead_code, reason = "feature gated")]
    Other(String),
}

impl DbError {
    pub(crate) fn skipped_ol_slot(expected: u64, got: u64) -> DbError {
        DbError::SkippedOlSlot { expected, got }
    }
}

#[cfg(feature = "sled")]
impl From<typed_sled::error::Error> for DbError {
    fn from(maybe_dberr: typed_sled::error::Error) -> Self {
        match maybe_dberr.downcast_abort::<DbError>() {
            Ok(dberr) => dberr,
            Err(other) => DbError::Sled(other.to_string()),
        }
    }
}

#[cfg(feature = "sled")]
impl From<sled::transaction::TransactionError<typed_sled::error::Error>> for DbError {
    fn from(value: sled::transaction::TransactionError<typed_sled::error::Error>) -> Self {
        match value {
            sled::transaction::TransactionError::Abort(tsled_err) => tsled_err.into(),
            err => DbError::SledTxn(err.to_string()),
        }
    }
}

#[cfg(feature = "rocksdb")]
impl From<rockbound::TransactionError<DbError>> for DbError {
    fn from(value: rockbound::TransactionError<DbError>) -> Self {
        match value {
            rockbound::TransactionError::Rollback(dberr) => dberr,
            err => DbError::RocksDBTxn(err.to_string()),
        }
    }
}

#[cfg(feature = "rocksdb")]
impl From<anyhow::Error> for DbError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value.to_string())
    }
}

impl From<DbError> for StorageError {
    fn from(err: DbError) -> Self {
        match err {
            // DbError::MissingAccountStateSlot(slot) => StorageError::StateNotFound(slot),
            DbError::SkippedOlSlot { expected, got } => StorageError::MissingSlot {
                attempted_slot: got,
                last_slot: expected,
            },
            e => StorageError::database(e.to_string()),
        }
    }
}
