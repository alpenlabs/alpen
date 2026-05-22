//! Error types for the checkpoint sync service.

use strata_db_types::DbError;
use strata_identifiers::Epoch;
use strata_primitives::EpochCommitment;
use thiserror::Error;

/// Errors from the checkpoint sync service.
#[derive(Debug, Error)]
pub(crate) enum CheckpointSyncError {
    /// A finalized epoch has no L1 observation entry in the database.
    #[error("finalized epoch {0} has no l1 observation entry")]
    MissingL1Ref(EpochCommitment),

    /// A finalized epoch's checkpoint is not buried deep enough to be
    /// reorg-safe, despite a descendant epoch being finalized.
    #[error("epoch {epoch} not reorg-safe: buried {depth} blocks, need {required}")]
    NotReorgSafe {
        epoch: EpochCommitment,
        depth: u32,
        required: u32,
    },

    /// An epoch's canonical predecessor is absent from the db (finalized chain hole).
    #[error("predecessor epoch {0} not found in db while scanning finalized chain")]
    MissingPredecessor(Epoch),

    /// A finalized epoch has no epoch summary when one was expected.
    #[error("epoch summary missing for {0}")]
    MissingEpochSummary(EpochCommitment),

    /// A database read failed.
    #[error("db: {0}")]
    Db(#[from] DbError),

    /// Failure from the chain worker while applying or finalizing an epoch.
    #[error("chain worker: {0}")]
    ChainWorker(#[source] anyhow::Error),
}

pub(crate) type CheckpointSyncResult<T> = Result<T, CheckpointSyncError>;
