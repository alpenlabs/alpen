//! Checks that ASM's active and pending VKs have matching loaded artifacts.
//!
//! These checks compare VKs only. Each proving task selects its artifact separately
//! from its epoch's terminal OL state.

use strata_identifiers::L1Height;
use strata_predicate::PredicateKey;

#[cfg(any(feature = "node", all(test, feature = "native")))]
use crate::LoadedCheckpointPredicates;

/// Describes whether a checkpoint VK is active or pending in ASM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointPredicateStatus {
    /// The active checkpoint predicate stored in ASM state.
    Active,
    /// An enacted VK waiting for ASM to accept a checkpoint ending at `boundary`.
    ///
    /// OL proving may already use this VK while it is pending in ASM.
    Pending { boundary: L1Height },
}

/// Reports an ASM VK that has no matching loaded artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingCheckpointArtifact {
    status: CheckpointPredicateStatus,
    predicate: PredicateKey,
}

impl MissingCheckpointArtifact {
    /// Returns whether the VK is active or pending in ASM.
    pub fn status(&self) -> CheckpointPredicateStatus {
        self.status
    }

    /// Returns the VK that has no matching loaded artifact.
    pub fn predicate(&self) -> &PredicateKey {
        &self.predicate
    }
}

/// Lists the missing artifacts that prevent prover startup.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("required checkpoint artifacts are missing: {missing_artifacts:?}")]
pub struct MissingCheckpointArtifacts {
    missing_artifacts: Vec<MissingCheckpointArtifact>,
}

impl MissingCheckpointArtifacts {
    /// Returns the active and pending VKs that have no matching loaded artifact.
    pub fn missing_artifacts(&self) -> &[MissingCheckpointArtifact] {
        &self.missing_artifacts
    }
}

/// Fails the startup check if any required VK has no matching loaded artifact.
#[cfg(any(feature = "node", all(test, feature = "native")))]
pub(crate) fn ensure_artifacts_available(
    missing_artifacts: &[MissingCheckpointArtifact],
) -> Result<(), MissingCheckpointArtifacts> {
    if missing_artifacts.is_empty() {
        Ok(())
    } else {
        Err(MissingCheckpointArtifacts {
            missing_artifacts: missing_artifacts.to_vec(),
        })
    }
}

/// Reports the VK if no loaded artifact matches it.
#[cfg(any(feature = "node", all(test, feature = "native")))]
pub(crate) fn check_predicate_artifact(
    predicates: &LoadedCheckpointPredicates,
    status: CheckpointPredicateStatus,
    predicate: &PredicateKey,
) -> Option<MissingCheckpointArtifact> {
    (!predicates.iter().any(|(_, resident)| resident == predicate)).then(|| {
        MissingCheckpointArtifact {
            status,
            predicate: predicate.clone(),
        }
    })
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use strata_ol_params::OLRuntimeParams;
    use strata_ol_state_types::OLSpecId;

    use super::*;
    use crate::native_checkpoint_registry;

    #[test]
    fn loaded_predicate_satisfies_active_and_pending_requirements() {
        let registry = native_checkpoint_registry(OLRuntimeParams::test_default());
        let predicates = registry.to_predicates();
        let resident = registry.get(OLSpecId::V1).unwrap().predicate();
        for status in [
            CheckpointPredicateStatus::Active,
            CheckpointPredicateStatus::Pending { boundary: 10 },
        ] {
            assert_eq!(
                check_predicate_artifact(&predicates, status, resident),
                None
            );
        }
        assert_eq!(ensure_artifacts_available(&[]), Ok(()));
    }

    #[test]
    fn startup_reports_all_missing_active_and_pending_predicates() {
        let registry = native_checkpoint_registry(OLRuntimeParams::test_default());
        let predicates = registry.to_predicates();
        let unknown = PredicateKey::always_accept();
        let missing_artifacts: Vec<_> = [
            CheckpointPredicateStatus::Active,
            CheckpointPredicateStatus::Pending { boundary: 10 },
        ]
        .into_iter()
        .filter_map(|status| check_predicate_artifact(&predicates, status, &unknown))
        .collect();
        let error = ensure_artifacts_available(&missing_artifacts).unwrap_err();
        assert_eq!(
            error.missing_artifacts(),
            &[
                MissingCheckpointArtifact {
                    status: CheckpointPredicateStatus::Active,
                    predicate: unknown.clone(),
                },
                MissingCheckpointArtifact {
                    status: CheckpointPredicateStatus::Pending { boundary: 10 },
                    predicate: unknown,
                },
            ]
        );
    }
}
