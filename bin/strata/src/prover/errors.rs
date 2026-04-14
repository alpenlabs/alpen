//! Error types for the integrated prover service.

use strata_db_types::DbError;
use strata_identifiers::EpochCommitment;
use strata_paas::ProverError as PaasError;

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

    #[error("DA state diff computation failed: {0}")]
    DaComputation(String),
}

/// Classifies input-fetch failures as retriable or permanent for the paas
/// service.
///
/// Stale commitments and missing epoch metadata reflect expected race
/// conditions — the orchestration layer resubmits the canonical epoch on
/// its next tick, so those retry. Anything else is treated as permanent.
impl From<ProverError> for PaasError {
    fn from(e: ProverError) -> Self {
        match e {
            ProverError::StaleTaskCommitment { .. }
            | ProverError::EpochCommitmentNotFound(_)
            | ProverError::EpochSummaryNotFound(_) => PaasError::TransientFailure(e.to_string()),
            _ => PaasError::PermanentFailure(e.to_string()),
        }
    }
}
