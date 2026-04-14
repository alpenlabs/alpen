//! Error types for the integrated prover service.

use strata_db_types::DbError;
use strata_identifiers::EpochCommitment;

/// Errors that can occur during proof input fetching.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ProverError {
    #[error("epoch summary not found for epoch index {0}")]
    EpochSummaryNotFound(u64),

    #[error("epoch commitment not found for epoch index {0}")]
    EpochCommitmentNotFound(u64),

    #[error(
        "stale checkpoint task commitment for epoch index {epoch}: task={task:?}, canonical={canonical:?}"
    )]
    StaleTaskCommitment {
        epoch: u64,
        task: EpochCommitment,
        canonical: EpochCommitment,
    },

    #[error("block not found at slot {0}")]
    BlockNotFound(u64),

    #[error("state not found for block commitment {0:?}")]
    StateNotFound(String),

    #[error("database error: {0}")]
    Database(#[from] DbError),

    #[error("proof input task join failed: {0}")]
    InputFetchJoin(String),

    #[error("DA state diff computation failed: {0}")]
    DaComputation(String),

    #[error("unsupported zkVM backend: {0}")]
    UnsupportedBackend(String),
}

/// Error wrapper for proof storage operations.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) struct ProofStorageError(#[from] pub(crate) anyhow::Error);
