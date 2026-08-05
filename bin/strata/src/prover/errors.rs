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
/// Missing epoch metadata is a genuine not-ready-yet wait — the epoch's
/// commitment/summary hasn't been produced yet — so those become transient
/// (the paas layer parks them as `Blocked` and rechecks).
///
/// A *stale* commitment is different: the task was submitted for an epoch
/// commitment that is no longer canonical (a same-epoch reorg replaced it). It
/// can never become canonical again, so retrying/reblocking it forever would
/// hang the checkpoint runner, which awaits `execute` inline. Mark it permanent
/// (→ `Rejected`) so `execute` returns `Failed` and the runner's recovery path
/// re-reads the now-canonical commitment and resubmits the replacement task.
impl From<ProverError> for PaasError {
    fn from(e: ProverError) -> Self {
        match e {
            ProverError::EpochCommitmentNotFound(_) | ProverError::EpochSummaryNotFound(_) => {
                PaasError::transient(e.to_string())
            }
            ProverError::StaleTaskCommitment { .. } => PaasError::permanent(e.to_string()),
            // Infra: surface as a retryable error, not a domain verdict.
            ProverError::Database(_) => PaasError::Storage(e.to_string()),
            _ => PaasError::permanent(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_paas::FailureAction;

    use super::*;

    fn action_of(e: ProverError) -> FailureAction {
        PaasError::from(e).action()
    }

    #[test]
    fn missing_epoch_metadata_is_transient() {
        // Not produced yet — the OL will fill it in; park as Blocked and recheck.
        assert_eq!(
            action_of(ProverError::EpochCommitmentNotFound(7)),
            FailureAction::RetryResume
        );
        assert_eq!(
            action_of(ProverError::EpochSummaryNotFound(7)),
            FailureAction::RetryResume
        );
    }

    #[test]
    fn stale_commitment_is_permanent() {
        // A same-epoch reorg replaced the commitment; it can never become
        // canonical again. Must be permanent (-> Rejected) so the checkpoint
        // runner's inline `execute` returns instead of blocking forever, letting
        // it re-read the canonical commitment and resubmit.
        assert_eq!(
            action_of(ProverError::StaleTaskCommitment {
                epoch: 3,
                task: EpochCommitment::null(),
                canonical: EpochCommitment::null(),
            }),
            FailureAction::Permanent
        );
    }

    #[test]
    fn database_errors_surface_as_storage() {
        assert_eq!(
            action_of(ProverError::DaComputation("boom".into())),
            FailureAction::Permanent
        );
    }
}
