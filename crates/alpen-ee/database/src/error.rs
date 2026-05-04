use alpen_ee_common::{BatchId, ChunkId, StorageError};
use sled::transaction::TransactionError;
use strata_acct_types::Hash;
use strata_identifiers::OLBlockId;
use strata_storage_common::exec::OpsError;
use thiserror::Error;
use typed_sled::error::Error as SledError;

pub type DbResult<T> = Result<T, DbError>;

/// Database-specific errors.
#[derive(Debug, Clone, Error)]
pub enum DbError {
    /// Attempted to persist a null OL block.
    #[error("null OL block should not be persisted")]
    NullOLBlock,

    /// OL slot was skipped in sequential persistence.
    #[error("OL entries must be persisted sequentially but provided nonsequentially (exp next {expected}, got {got})")]
    SkippedOLSlot { expected: u64, got: u64 },

    /// Transaction conflict: slot is already filled.
    #[error("likely db txn conflict, OL slot {0} already filled")]
    TxnFilledOLSlot(u64),

    /// Transaction conflict: expected slot to be empty.
    #[error("likely db txn conflict, OL slot {0} should be empty")]
    TxnExpectEmptyOLSlot(u64),

    /// Account state is missing for the given block.
    #[error("account state missing (at blkid {0})")]
    MissingAccountState(OLBlockId),

    /// Finalized chain is empty.
    #[error("finalized exec block expected to be present")]
    FinalizedExecChainEmpty,

    /// Exec block is missing.
    #[error("missing expected exec blkid {0}")]
    MissingExecBlock(Hash),

    #[error("expected exec block finalized chain to be empty")]
    FinalizedExecChainGenesisBlockMismatch,

    #[error("provided blkid {0} does not extend chain")]
    ExecBlockDoesNotExtendChain(Hash),

    /// Walk from `new_tip` failed to reach the current finalized tip.
    ///
    /// This indicates one of:
    /// - `new_tip` is non-canonical (does not descend from finalized tip),
    /// - storage inconsistency (parent links / block numbers disagree), or
    /// - walk exceeded the expected height-difference budget without reaching the tip (e.g.
    ///   cyclic/corrupt parent links above finalized height).
    #[error("walk failed to reach finalized tip (new tip {new_tip}, finalized height {finalized_height})")]
    FinalizedWalkNotDescending {
        new_tip: Hash,
        finalized_height: u64,
    },

    /// Walk from `new_tip` did not reach finalized tip within expected height-difference budget.
    ///
    /// This usually indicates cyclic/corrupt parent links above finalized height, or severe
    /// block-number/parent inconsistency that prevented convergence.
    #[error("walk exhausted step budget before reaching tip (new tip {new_tip}, finalized height {finalized_height}, max steps {max_steps})")]
    FinalizedWalkStepBudgetExceeded {
        new_tip: Hash,
        finalized_height: u64,
        max_steps: u64,
    },

    #[error("likely db txn conflict, expected finalized height {0} to be empty")]
    TxnExpectEmptyFinalized(u64),

    #[error("likely db txn conflict, expected finalized height {0} to be {1}")]
    TxnExpectFinalized(u64, Hash),

    /// Attempted to delete a finalized block.
    #[error("tried to delete finalized block {0}")]
    CannotDeleteFinalizedBlock(Hash),

    /// Batch not found when trying to update status.
    #[error("batch {0} not found")]
    BatchNotFound(BatchId),

    /// Chunk not found when trying to update status.
    // NOTE: `ChunkId` doesn't implement `Display`, so use `Debug` here.
    #[error("chunk {0:?} not found")]
    ChunkNotFound(ChunkId),

    /// Batch deserialization error.
    #[error("Failed to deserialize batch: {0}")]
    BatchDeserialize(String),

    /// Database operation error.
    #[error("db ops: {0}")]
    DbOpsError(#[from] OpsError),

    /// Sled database error.
    #[error("sled: {0}")]
    Sled(String),

    /// Sled transaction error.
    #[error("sled txn: {0}")]
    SledTxn(String),

    /// Other unspecified database error.
    #[error("{0}")]
    Other(String),
}

impl DbError {
    pub(crate) fn skipped_ol_slot(expected: u64, got: u64) -> DbError {
        DbError::SkippedOLSlot { expected, got }
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
            DbError::SkippedOLSlot { expected, got } => StorageError::MissingSlot {
                attempted_slot: got,
                last_slot: expected,
            },
            DbError::CannotDeleteFinalizedBlock(hash) => {
                StorageError::CannotDeleteFinalizedBlock(format!("{:?}", hash))
            }
            e => StorageError::database(e.to_string()),
        }
    }
}
