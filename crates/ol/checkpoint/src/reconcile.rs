//! Finds a canonical starting point for checkpoint consumers after a reorg.

use std::sync::Arc;

use strata_db_types::DbError;
use strata_identifiers::EpochCommitment;
use strata_ol_canonical_chain_provider::{OLCanonicalChainProvider, OLCanonicalChainProviderImpl};
use strata_storage::{NodeStorage, OLCheckpointManager};

/// Failures to reconcile processed epoch history.
#[derive(Debug, thiserror::Error)]
pub enum CanonicalEpochReconcileError {
    #[error("canonical epoch reconciliation read failed: {0}")]
    Storage(#[from] DbError),

    #[error("missing processed epoch summary for {0}")]
    MissingSummary(EpochCommitment),

    #[error("epoch summary does not match commitment {expected} (got {actual})")]
    SummaryMismatch {
        expected: EpochCommitment,
        actual: EpochCommitment,
    },

    #[error("epoch summary has invalid predecessor {previous} for {commitment}")]
    InvalidPredecessor {
        commitment: EpochCommitment,
        previous: EpochCommitment,
    },

    #[error("canonical history conflicts with retained anchor {0}")]
    HistoryBaseConflict(EpochCommitment),
}

/// Reconciles epoch history using canonical index reads and exact stored summaries.
#[expect(
    missing_debug_implementations,
    reason = "storage managers do not implement Debug"
)]
#[derive(Clone)]
pub struct CanonicalEpochReconciler {
    canonical_chain_provider: OLCanonicalChainProviderImpl,
    checkpoint_mgr: Arc<OLCheckpointManager>,
}

impl CanonicalEpochReconciler {
    /// Creates a reconciler using managers from the same node storage.
    pub fn new(storage: &NodeStorage) -> Self {
        Self {
            canonical_chain_provider: OLCanonicalChainProviderImpl::new(storage),
            checkpoint_mgr: storage.ol_checkpoint().clone(),
        }
    }

    /// Finds the latest canonical commitment at or before `last_processed` on its branch.
    ///
    /// Returns `last_processed` itself if it is canonical. Without a history base,
    /// reaching epoch zero returns `None`, meaning resume at epoch one. A history
    /// base is checked against the canonical block at its slot without requiring a summary.
    /// Missing or inconsistent epoch summaries and a conflicting base are errors.
    ///
    /// Performs blocking reads. The result reflects canonicality when read;
    /// it does not prevent subsequent reorgs or advance the consumer's progress.
    pub fn find_canonical_resume_point(
        &self,
        last_processed: EpochCommitment,
        history_base: Option<EpochCommitment>,
    ) -> Result<Option<EpochCommitment>, CanonicalEpochReconcileError> {
        let mut commitment = last_processed;
        loop {
            if let Some(base) = history_base {
                if commitment.epoch() < base.epoch()
                    || (commitment.epoch() == base.epoch() && commitment != base)
                    || (commitment.epoch() > base.epoch()
                        && commitment.last_slot() <= base.last_slot())
                {
                    return Err(CanonicalEpochReconcileError::HistoryBaseConflict(base));
                }
                if commitment == base {
                    let canonical = self
                        .canonical_chain_provider
                        .get_canonical_block_at_blocking(base.last_slot())?;
                    if canonical != Some(base.to_block_commitment()) {
                        return Err(CanonicalEpochReconcileError::HistoryBaseConflict(base));
                    }
                    return Ok(Some(base));
                }
            } else if commitment.epoch() == 0 {
                return Ok(None);
            }

            if self
                .canonical_chain_provider
                .get_canonical_epoch_commitment_at_blocking(commitment.epoch())?
                == Some(commitment)
            {
                return Ok(Some(commitment));
            }
            let summary = self
                .checkpoint_mgr
                .get_epoch_summary_blocking(commitment)?
                .ok_or(CanonicalEpochReconcileError::MissingSummary(commitment))?;
            let actual = summary.get_epoch_commitment();
            if actual != commitment {
                return Err(CanonicalEpochReconcileError::SummaryMismatch {
                    expected: commitment,
                    actual,
                });
            }
            // Epoch zero either returned above or conflicts with the supplied base.
            let previous =
                EpochCommitment::from_terminal(commitment.epoch() - 1, *summary.prev_terminal());
            if previous.last_slot() >= commitment.last_slot() {
                return Err(CanonicalEpochReconcileError::InvalidPredecessor {
                    commitment,
                    previous,
                });
            }
            commitment = previous;
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_checkpoint_types::EpochSummary;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{Buf32, Epoch, L1BlockCommitment, OLBlockCommitment, OLBlockId, Slot};
    use strata_storage::{create_node_storage, test_runtime_handle};
    use tokio::task::spawn_blocking;

    use super::*;

    struct Fixture {
        storage: NodeStorage,
        reconciler: CanonicalEpochReconciler,
    }

    impl Fixture {
        fn new() -> Self {
            let storage =
                create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap();
            let reconciler = CanonicalEpochReconciler::new(&storage);
            Self {
                storage,
                reconciler,
            }
        }

        fn store_epoch(&self, previous: EpochCommitment, seed: u8) -> EpochCommitment {
            let current = commitment(previous.epoch() + 1, previous.last_slot() + 1, seed);
            self.store_summary(current, previous.to_block_commitment());
            current
        }

        fn store_summary(&self, current: EpochCommitment, previous: OLBlockCommitment) {
            let summary = EpochSummary::new(
                current.epoch(),
                current.to_block_commitment(),
                previous,
                L1BlockCommitment::default(),
                Buf32::zero(),
            );
            self.storage
                .ol_checkpoint()
                .insert_epoch_summary_blocking(summary)
                .unwrap();
        }

        fn publish(&self, current: EpochCommitment) {
            self.storage
                .ol_block()
                .replace_canonical_suffix_from_blocking(
                    current.last_slot(),
                    vec![*current.last_blkid()],
                )
                .unwrap();
        }
    }

    fn commitment(epoch: Epoch, slot: Slot, seed: u8) -> EpochCommitment {
        EpochCommitment::new(epoch, slot, OLBlockId::from(Buf32::from([seed; 32])))
    }

    #[test]
    fn test_canonical_position_does_not_walk_earlier_summaries() {
        let fixture = Fixture::new();
        let previous = commitment(3, 10, 1);
        let current = fixture.store_epoch(previous, 2);
        fixture.publish(current);
        // Earlier summaries and headers are deliberately absent.
        assert_eq!(
            fixture
                .reconciler
                .find_canonical_resume_point(current, None)
                .unwrap(),
            Some(current)
        );
    }

    #[test]
    fn test_same_epoch_and_multi_epoch_replacements_find_common_predecessor() {
        let fixture = Fixture::new();
        let first = fixture.store_epoch(EpochCommitment::null(), 1);
        let second = fixture.store_epoch(first, 2);
        let third = fixture.store_epoch(second, 3);
        fixture.publish(first);
        fixture.publish(second);
        fixture.publish(third);

        let replacement_third = fixture.store_epoch(second, 4);
        fixture.publish(replacement_third);
        assert_eq!(
            fixture
                .reconciler
                .find_canonical_resume_point(third, None)
                .unwrap(),
            Some(second)
        );

        let replacement_second = fixture.store_epoch(first, 5);
        fixture.publish(replacement_second);
        assert_eq!(
            fixture
                .reconciler
                .find_canonical_resume_point(third, None)
                .unwrap(),
            Some(first)
        );
    }

    #[test]
    fn test_removed_canonical_suffix_uses_old_branch_summaries() {
        let fixture = Fixture::new();
        let first = fixture.store_epoch(EpochCommitment::null(), 1);
        let second = fixture.store_epoch(first, 2);
        fixture.publish(first);
        fixture.publish(second);
        fixture
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(second.last_slot(), vec![])
            .unwrap();
        assert_eq!(
            fixture
                .reconciler
                .find_canonical_resume_point(second, None)
                .unwrap(),
            Some(first)
        );
    }

    #[test]
    fn test_genesis_without_anchor_resumes_at_epoch_one() {
        let fixture = Fixture::new();
        let genesis = commitment(0, 0, 1);
        let old = fixture.store_epoch(genesis, 2);
        let replacement = fixture.store_epoch(genesis, 3);
        fixture.publish(replacement);
        assert_eq!(
            fixture
                .reconciler
                .find_canonical_resume_point(old, None)
                .unwrap(),
            None
        );
        assert_eq!(
            fixture
                .reconciler
                .find_canonical_resume_point(genesis, None)
                .unwrap(),
            None
        );
    }

    #[test]
    fn test_history_anchor_needs_only_its_canonical_row() {
        for epoch in [0, 5] {
            let fixture = Fixture::new();
            let base = commitment(epoch, u64::from(epoch), 1);
            let old = fixture.store_epoch(base, 2);
            let replacement = fixture.store_epoch(base, 3);
            // No anchor summary, header, or state is required by reconciliation.
            fixture.publish(base);
            fixture.publish(replacement);
            assert_eq!(
                fixture
                    .reconciler
                    .find_canonical_resume_point(old, Some(base))
                    .unwrap(),
                Some(base)
            );
            assert_eq!(
                fixture
                    .reconciler
                    .find_canonical_resume_point(base, Some(base))
                    .unwrap(),
                Some(base)
            );
        }
    }

    #[test]
    fn test_missing_or_replaced_anchor_errors() {
        let fixture = Fixture::new();
        let base = commitment(5, 10, 1);
        assert!(
            matches!(fixture.reconciler.find_canonical_resume_point(base, Some(base)),
            Err(CanonicalEpochReconcileError::HistoryBaseConflict(found)) if found == base)
        );
        fixture.publish(commitment(5, 10, 2));
        assert!(
            matches!(fixture.reconciler.find_canonical_resume_point(base, Some(base)),
            Err(CanonicalEpochReconcileError::HistoryBaseConflict(found)) if found == base)
        );
    }

    #[test]
    fn test_cursor_conflicting_with_history_base_rejected() {
        // Simulates corrupted state; the history-base guard must run before the canonicality check.
        let base = commitment(5, 10, 1);
        for current in [
            commitment(4, 9, 2),
            commitment(5, 10, 2),
            commitment(6, 10, 2),
        ] {
            let fixture = Fixture::new();
            fixture.store_summary(current, OLBlockCommitment::null());
            fixture.publish(current);
            assert!(
                matches!(fixture.reconciler.find_canonical_resume_point(current, Some(base)),
                Err(CanonicalEpochReconcileError::HistoryBaseConflict(found)) if found == base)
            );
        }
    }

    #[test]
    fn test_old_branch_cannot_substitute_another_anchor() {
        let fixture = Fixture::new();
        let base = commitment(5, 10, 1);
        let other_base = commitment(5, 10, 2);
        let old = fixture.store_epoch(other_base, 3);
        let replacement = fixture.store_epoch(base, 4);
        fixture.publish(base);
        fixture.publish(replacement);
        assert!(
            matches!(fixture.reconciler.find_canonical_resume_point(old, Some(base)),
            Err(CanonicalEpochReconcileError::HistoryBaseConflict(found)) if found == base)
        );
    }

    #[test]
    fn test_missing_processed_summary_errors() {
        let fixture = Fixture::new();
        let missing = commitment(2, 4, 1);
        let last = fixture.store_epoch(missing, 2);
        assert!(
            matches!(fixture.reconciler.find_canonical_resume_point(last, None),
            Err(CanonicalEpochReconcileError::MissingSummary(found)) if found == missing)
        );
    }

    #[test]
    fn test_predecessor_slot_must_decrease() {
        for previous_slot in [10, 11] {
            let fixture = Fixture::new();
            let current = commitment(3, 10, 1);
            let previous = commitment(2, previous_slot, 2);
            fixture.store_summary(current, previous.to_block_commitment());
            assert!(
                matches!(fixture.reconciler.find_canonical_resume_point(current, None),
                Err(CanonicalEpochReconcileError::InvalidPredecessor { commitment: found, previous: parent })
                    if found == current && parent == previous)
            );
        }
    }

    #[test]
    fn test_reconciler_can_run_on_a_blocking_task() {
        let fixture = Fixture::new();
        let base = commitment(5, 10, 1);
        let old = fixture.store_epoch(base, 2);
        fixture.publish(base);
        let reconciler = fixture.reconciler.clone();
        let result = test_runtime_handle().block_on(async move {
            spawn_blocking(move || reconciler.find_canonical_resume_point(old, Some(base)))
                .await
                .unwrap()
        });
        assert_eq!(result.unwrap(), Some(base));
    }
}
