//! Error types for the checkpoint sync service.

use strata_chain_worker::WorkerError;
use strata_db_types::DbError;
use strata_identifiers::Epoch;
use strata_primitives::{EpochCommitment, OLBlockCommitment};
use thiserror::Error;

use crate::ol_mmr_reconcile::OLMmrReconcileError;

/// Errors from the checkpoint sync service.
#[derive(Debug, Error)]
pub enum CheckpointSyncError {
    /// L1 canonical chain tip not yet available (pre-sync); treat as wait.
    #[error("L1 canonical chain tip not yet ingested")]
    L1TipNotReady,

    /// A finalized epoch has no L1 observation entry in the database.
    #[error("finalized epoch {0} has no l1 observation entry")]
    MissingL1Ref(EpochCommitment),

    /// A finalized epoch's checkpoint is not buried deep enough to be
    /// reorg-safe, despite a descendant epoch being finalized.
    #[error("epoch {epoch} not reorg-safe (buried {depth} blocks, need {required})")]
    NotReorgSafe {
        epoch: EpochCommitment,
        depth: u32,
        required: u32,
    },

    /// An epoch's canonical predecessor is absent from the db (finalized chain hole).
    #[error("predecessor epoch {0} not found in db while scanning finalized chain")]
    MissingPredecessor(Epoch),

    /// More than one checkpoint observation for an epoch survives the canonical-L1
    /// filter, so the canonical one is ambiguous.
    #[error("multiple canonical checkpoint observations for epoch {0}")]
    AmbiguousObservation(Epoch),

    /// A finalized epoch has no epoch summary when one was expected.
    #[error("epoch summary missing for {0}")]
    MissingEpochSummary(EpochCommitment),

    /// A summarized epoch has no canonical epoch commitment.
    #[error("canonical epoch commitment missing for summarized epoch {0}")]
    MissingCanonicalEpochCommitment(Epoch),

    /// A target block has no persisted OL state.
    #[error("missing OL state for block {0}")]
    MissingOLState(OLBlockCommitment),

    /// A database read failed.
    #[error("db: {0}")]
    Db(#[from] DbError),

    /// Failure from a per-epoch chain-worker call (`apply`, `finalize`).
    #[error("chain worker {op} at {epoch}: {cause}")]
    EpochOp {
        epoch: EpochCommitment,
        op: &'static str,
        #[source]
        cause: WorkerError,
    },

    /// Failure updating the chain worker's safe tip.
    #[error("chain worker update_safe_tip: {0}")]
    SafeTipUpdate(#[source] WorkerError),

    /// Failure reconciling OL-owned MMR indexes before checkpoint sync resumes.
    #[error("OL MMR index reconciliation: {0}")]
    OLMmrReconcile(#[from] OLMmrReconcileError),
}

pub type CheckpointSyncResult<T> = Result<T, CheckpointSyncError>;
