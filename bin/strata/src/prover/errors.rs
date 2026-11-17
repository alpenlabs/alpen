//! Error types for the integrated prover service.

use strata_db_types::DbError;
use strata_identifiers::{EpochCommitment, OLBlockCommitment};
use strata_ol_block_ancestry_provider::BlockAncestryError;
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

    #[error("block header not found for {block}")]
    MissingBlockHeader { block: OLBlockCommitment },

    #[error("epoch block not found for {block}")]
    MissingEpochBlock { block: OLBlockCommitment },

    #[error("database error: {0}")]
    Database(#[from] DbError),

    #[error("block ancestry lookup failed: {0}")]
    BlockAncestry(#[from] BlockAncestryError),

    #[error("epoch commitment mismatch (expected {expected}, got {actual})")]
    EpochCommitmentMismatch {
        expected: EpochCommitment,
        actual: EpochCommitment,
    },

    #[error("loaded block does not match ancestry commitment (expected {expected}, got {actual})")]
    BlockCommitmentMismatch {
        expected: OLBlockCommitment,
        actual: OLBlockCommitment,
    },

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
/// Missing or inconsistent local block history for a canonical task is a storage failure and is
/// retried after repair. This includes ancestry, headers and epoch block bodies. An epoch-zero
/// start is permanent because epoch zero is never proved.
///
/// A stale commitment is permanent. The task's branch has lost fork choice, and
/// input resolution rejects it before backend work starts so an abandoned branch
/// is not proven. Retrying would only reconsume budget while the branch stays
/// noncanonical. If the branch later becomes canonical again, that exact task
/// stays rejected; reviving it is tracked by STR-4610.
impl From<ProverError> for PaasError {
    fn from(e: ProverError) -> Self {
        let message = e.to_string();
        match e {
            ProverError::EpochCommitmentNotFound(_)
            | ProverError::EpochSummaryNotFound(_)
            | ProverError::EpochStartStateNotFound { .. }
            | ProverError::EpochTerminalStateNotFound { .. } => PaasError::transient(message),
            ProverError::StaleTaskCommitment { .. } => PaasError::permanent(message),
            // Local storage failures are retryable after repair.
            ProverError::Database(_)
            | ProverError::MissingBlockHeader { .. }
            | ProverError::MissingEpochBlock { .. }
            | ProverError::BlockCommitmentMismatch { .. } => PaasError::Storage(message),
            ProverError::BlockAncestry(error) => match error {
                BlockAncestryError::Db(_)
                | BlockAncestryError::MissingEpochTerminal(_)
                | BlockAncestryError::EpochTerminalMismatch { .. }
                | BlockAncestryError::MissingTerminalFlag(_)
                | BlockAncestryError::PreviousTerminalMismatch { .. }
                | BlockAncestryError::MissingHeader(_)
                | BlockAncestryError::BlockIdMismatch { .. }
                | BlockAncestryError::UnexpectedAncestor { .. }
                | BlockAncestryError::ParentSlotMismatch { .. }
                | BlockAncestryError::UnexpectedEpoch { .. } => PaasError::Storage(message),
                BlockAncestryError::EpochZeroHasNoPreviousEpoch(_) => PaasError::permanent(message),
            },
            _ => PaasError::permanent(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_identifiers::{Buf32, OLBlockId};
    use strata_paas::FailureAction;

    use super::*;

    fn action_of(e: ProverError) -> FailureAction {
        PaasError::from(e).action()
    }

    #[test]
    fn missing_epoch_metadata_maps_to_retry_resume() {
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
    fn stale_commitment_maps_to_permanent_failure() {
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
    fn summary_commitment_mismatch_maps_to_permanent_failure() {
        let expected_terminal = OLBlockCommitment::new(7, OLBlockId::from(Buf32::from([1; 32])));
        let actual_terminal = OLBlockCommitment::new(8, OLBlockId::from(Buf32::from([2; 32])));
        let expected = EpochCommitment::from_terminal(2, expected_terminal);
        let actual = EpochCommitment::from_terminal(3, actual_terminal);
        assert_eq!(
            action_of(ProverError::EpochCommitmentMismatch { expected, actual }),
            FailureAction::Permanent
        );
    }

    #[test]
    fn block_commitment_mismatch_maps_to_storage_error() {
        let expected = OLBlockCommitment::new(7, OLBlockId::from(Buf32::from([1; 32])));
        let actual = OLBlockCommitment::new(expected.slot(), OLBlockId::from(Buf32::from([2; 32])));
        assert!(matches!(
            PaasError::from(ProverError::BlockCommitmentMismatch { expected, actual }),
            PaasError::Storage(_)
        ));
    }

    #[test]
    fn repairable_ancestry_failures_map_to_storage_errors() {
        let block = OLBlockCommitment::new(7, OLBlockId::from(Buf32::from([1; 32])));
        let other_id = OLBlockId::from(Buf32::from([2; 32]));
        let expected_epoch = EpochCommitment::from_terminal(2, block);
        let actual_terminal = OLBlockCommitment::new(8, other_id);
        let actual_epoch = EpochCommitment::from_terminal(3, actual_terminal);
        for error in [
            BlockAncestryError::Db(DbError::NotBootstrapped),
            BlockAncestryError::MissingEpochTerminal(expected_epoch),
            BlockAncestryError::EpochTerminalMismatch {
                expected: expected_epoch,
                actual: actual_epoch,
            },
            BlockAncestryError::MissingTerminalFlag(expected_epoch),
            BlockAncestryError::PreviousTerminalMismatch {
                expected: block,
                actual: actual_terminal,
            },
            BlockAncestryError::MissingHeader(*block.blkid()),
            BlockAncestryError::BlockIdMismatch {
                expected: *block.blkid(),
                actual: other_id,
            },
            BlockAncestryError::UnexpectedAncestor {
                expected: *block.blkid(),
                actual: other_id,
            },
            BlockAncestryError::ParentSlotMismatch {
                child: block,
                parent_slot: block.slot() + 1,
            },
            BlockAncestryError::UnexpectedEpoch {
                block,
                expected: 1,
                actual: 2,
            },
        ] {
            assert!(matches!(
                PaasError::from(ProverError::from(error)),
                PaasError::Storage(_)
            ));
        }
    }

    #[test]
    fn epoch_zero_ancestry_failure_maps_to_permanent_failure() {
        assert_eq!(
            action_of(ProverError::from(
                BlockAncestryError::EpochZeroHasNoPreviousEpoch(OLBlockCommitment::new(
                    0,
                    OLBlockId::from(Buf32::from([1; 32])),
                )),
            )),
            FailureAction::Permanent,
        );
    }

    #[test]
    fn da_computation_errors_map_to_permanent_failure() {
        assert_eq!(
            action_of(ProverError::DaComputation("boom".into())),
            FailureAction::Permanent
        );
    }

    #[test]
    fn missing_epoch_states_map_to_retry_resume() {
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
    }

    #[test]
    fn missing_local_block_data_maps_to_storage_errors() {
        let block = OLBlockCommitment::new(7, OLBlockId::from(Buf32::from([1; 32])));
        for error in [
            ProverError::MissingBlockHeader { block },
            ProverError::MissingEpochBlock { block },
        ] {
            assert!(matches!(PaasError::from(error), PaasError::Storage(_)));
        }
    }

    #[test]
    fn misrouted_task_maps_to_permanent_failure() {
        let error = ProverError::MisroutedTask {
            task: EpochCommitment::null(),
            assigned: OLSpecId::V0,
            required: OLSpecId::V1,
        };
        assert_eq!(action_of(error), FailureAction::Permanent);
    }
}
