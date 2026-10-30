//! Error types for the integrated prover service.

use strata_db_types::DbError;
use strata_identifiers::{EpochCommitment, OLBlockCommitment};
use strata_ol_checkpoint::CanonicalEpochReconcileError;
use strata_ol_state_types::OLSpecId;
use strata_paas::ProverError as PaasError;

/// Errors that can occur during proof input fetching.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ProverError {
    #[error("checkpoint task {task:?} requires {required:?}, but was routed to {assigned:?}")]
    MisroutedTask {
        task: EpochCommitment,
        assigned: OLSpecId,
        required: OLSpecId,
    },

    #[error("checkpoint epoch start state not found at {commitment:?}")]
    EpochStartStateNotFound { commitment: OLBlockCommitment },

    #[error("checkpoint epoch terminal state not found at {commitment:?}")]
    EpochTerminalStateNotFound { commitment: OLBlockCommitment },

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

/// Required checkpoint history is missing or inconsistent.
#[derive(Debug, thiserror::Error)]
pub(super) enum CheckpointHistoryError {
    #[error("missing epoch summary for {0}")]
    MissingSummary(EpochCommitment),
    #[error("epoch summary does not match commitment {expected} (got {actual})")]
    SummaryMismatch {
        expected: EpochCommitment,
        actual: EpochCommitment,
    },
    #[error(
        "epoch {commitment} predecessor does not match processed epoch {expected} (got {actual})"
    )]
    PredecessorMismatch {
        commitment: EpochCommitment,
        expected: EpochCommitment,
        actual: OLBlockCommitment,
    },
}

pub(super) fn is_history_integrity_failure(error: &anyhow::Error) -> bool {
    if error.is::<CheckpointHistoryError>() {
        return true;
    }
    match error.downcast_ref::<CanonicalEpochReconcileError>() {
        Some(
            CanonicalEpochReconcileError::MissingSummary(_)
            | CanonicalEpochReconcileError::SummaryMismatch { .. }
            | CanonicalEpochReconcileError::InvalidPredecessor { .. }
            | CanonicalEpochReconcileError::HistoryBaseConflict(_),
        ) => true,
        Some(CanonicalEpochReconcileError::Storage(_)) | None => false,
    }
}

/// Classifies input-fetch failures as retriable or permanent for the paas
/// service.
///
/// Missing epoch metadata or its exact start or terminal state is a not-ready-yet wait;
/// the paas layer parks those tasks as `Blocked` and rechecks.
///
/// A stale commitment is permanent. The task's branch has lost fork choice, and
/// input resolution rejects it before backend work starts so an abandoned branch
/// is not proven. Retrying would only reconsume budget while the branch stays
/// noncanonical. If the branch later becomes canonical again, that exact task
/// stays rejected; reviving it is tracked by STR-4610.
impl From<ProverError> for PaasError {
    fn from(e: ProverError) -> Self {
        match e {
            ProverError::EpochCommitmentNotFound(_)
            | ProverError::EpochSummaryNotFound(_)
            | ProverError::EpochStartStateNotFound { .. }
            | ProverError::EpochTerminalStateNotFound { .. } => PaasError::transient(e.to_string()),
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
        // Abandoned branches are rejected before proving; see the mapping's doc.
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

    #[test]
    fn missing_epoch_states_are_transient_but_chain_traversal_failure_is_not() {
        assert_eq!(
            action_of(ProverError::EpochStartStateNotFound {
                commitment: OLBlockCommitment::default(),
            }),
            FailureAction::RetryResume,
        );
        assert_eq!(
            action_of(ProverError::EpochTerminalStateNotFound {
                commitment: OLBlockCommitment::default(),
            }),
            FailureAction::RetryResume,
        );
        assert_eq!(
            action_of(ProverError::StateNotFound("missing epoch block".into())),
            FailureAction::Permanent,
        );
    }

    #[test]
    fn wrong_service_assignment_is_permanent() {
        let error = ProverError::MisroutedTask {
            task: EpochCommitment::null(),
            assigned: OLSpecId::V0,
            required: OLSpecId::V1,
        };
        assert_eq!(action_of(error), FailureAction::Permanent);
    }
}
