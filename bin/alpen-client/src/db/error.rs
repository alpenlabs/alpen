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

    #[error("OL slot {0} already filled")]
    FilledOlSlotTxn(u64),

    #[error("Account state expected to be present; block_id = {0}")]
    MissingAccountState(OLBlockId),

    #[error("Account state expected to be present; slot = {0}")]
    MissingAccountStateSlot(u64),

    #[error("Database: {0}")]
    DbOpsError(#[from] OpsError),

    #[cfg(feature = "sled")]
    #[error("sled: {0}")]
    Sled(String),

    #[cfg(feature = "sled")]
    #[error("sled txn: {0}")]
    SledTxn(String),
}

impl DbError {
    pub(crate) fn skipped_ol_slot(expected: u64, got: u64) -> DbError {
        DbError::SkippedOlSlot { expected, got }
    }
}

impl From<typed_sled::error::Error> for DbError {
    fn from(maybe_dberr: typed_sled::error::Error) -> Self {
        match maybe_dberr.downcast_abort::<DbError>() {
            Ok(dberr) => dberr,
            Err(other) => DbError::Sled(other.to_string()),
        }
    }
}

impl From<sled::transaction::TransactionError<typed_sled::error::Error>> for DbError {
    fn from(value: sled::transaction::TransactionError<typed_sled::error::Error>) -> Self {
        match value {
            sled::transaction::TransactionError::Abort(tsled_err) => tsled_err.into(),
            err => DbError::SledTxn(err.to_string()),
        }
    }
}

impl From<DbError> for StorageError {
    fn from(err: DbError) -> Self {
        match err {
            DbError::MissingAccountStateSlot(slot) => StorageError::StateNotFound(slot),
            e => StorageError::database(e.to_string()),
        }
    }
}

// impl From<OpsError> for DbError {
//     fn from(value: OpsError) -> Self {
//         DbError::Other(Arc::new(value))
//     }
// }
