//! Rejects outdated checkpoint tasks and checks that their prover has a loaded artifact.

use std::sync::Arc;

use async_trait::async_trait;
use strata_ol_checkpoint_artifacts::LoadedCheckpointPredicates;
use strata_ol_state_types::OLSpecId;
use strata_paas::{AdmissionDecision, ProverError as PaasError, ProverResult, TaskAdmission};
use strata_storage::NodeStorage;
use tokio::task::spawn_blocking;

use super::{
    errors::ProverError,
    spec::{CheckpointSpec, CheckpointTask, validate_checkpoint_task_commitment},
};

/// Checks whether a checkpoint task can start proving.
///
/// Runs for new tasks, retries, and tasks recovered after restart.
/// Rejects tasks that no longer match the canonical checkpoint. If this service's
/// artifact is missing, the task waits for the operator to configure it and restart
/// the prover.
pub(crate) struct CheckpointAdmission {
    storage: Arc<NodeStorage>,
    predicates: Arc<LoadedCheckpointPredicates>,
    assigned_spec: OLSpecId,
}

impl CheckpointAdmission {
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        predicates: LoadedCheckpointPredicates,
        assigned_spec: OLSpecId,
    ) -> Self {
        Self {
            storage,
            predicates: Arc::new(predicates),
            assigned_spec,
        }
    }
}

#[async_trait]
impl TaskAdmission<CheckpointSpec> for CheckpointAdmission {
    async fn check(&self, task: &CheckpointTask) -> ProverResult<AdmissionDecision> {
        let storage = Arc::clone(&self.storage);
        let predicates = Arc::clone(&self.predicates);
        let assigned_spec = self.assigned_spec;
        let task = *task;
        spawn_blocking(move || {
            // Reject an outdated task before checking artifacts, so the runner can
            // replace it instead of leaving it waiting for a missing artifact.
            match validate_checkpoint_task_commitment(&storage, task.0) {
                Ok(()) => {}
                // Let input resolution wait for missing epoch metadata.
                Err(ProverError::EpochCommitmentNotFound(_)) => {
                    return Ok(AdmissionDecision::Admit);
                }
                Err(error) => return Err(error.into()),
            }

            if predicates.predicate(assigned_spec).is_none() {
                return Ok(AdmissionDecision::AwaitingConfiguration {
                    reason: format!(
                        "no checkpoint artifact is configured for OL spec {assigned_spec:?}"
                    ),
                    recheck_after: None,
                });
            }
            Ok(AdmissionDecision::Admit)
        })
        .await
        .map_err(|error| PaasError::Storage(format!("checkpoint admission join: {error}")))?
    }
}
