use std::sync::Arc;

use strata_db_types::DbResult;
use strata_identifiers::{Epoch, EpochCommitment, OLBlockCommitment, OLBlockId, Slot};
use strata_storage::{NodeStorage, OLBlockManager, OLCheckpointManager};

use crate::OLCanonicalChainProvider;

/// Resolves canonical commitments using the block index and stored epoch candidates.
#[expect(
    missing_debug_implementations,
    reason = "storage managers do not implement Debug"
)]
#[derive(Clone)]
pub struct OLCanonicalChainProviderImpl {
    blocks: Arc<OLBlockManager>,
    checkpoints: Arc<OLCheckpointManager>,
}

impl OLCanonicalChainProviderImpl {
    /// Creates a provider using the block and checkpoint managers from `storage`.
    pub fn new(storage: &NodeStorage) -> Self {
        Self {
            blocks: storage.ol_block().clone(),
            checkpoints: storage.ol_checkpoint().clone(),
        }
    }
}

impl OLCanonicalChainProvider for OLCanonicalChainProviderImpl {
    /// Returns `None` when the canonical index has no entry at `slot`.
    async fn get_canonical_block_at_async(
        &self,
        slot: Slot,
    ) -> DbResult<Option<OLBlockCommitment>> {
        self.blocks.get_canonical_block_at_async(slot).await
    }

    fn get_canonical_block_at_blocking(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>> {
        self.blocks.get_canonical_block_at_blocking(slot)
    }

    /// Returns `None` when the canonical index is empty.
    ///
    /// The slot and block ID are read together, even during a concurrent canonical
    /// suffix replacement.
    async fn get_canonical_tip_async(&self) -> DbResult<Option<OLBlockCommitment>> {
        self.blocks.get_canonical_tip_async().await
    }

    /// Blocking counterpart of [`Self::get_canonical_tip_async`], with the same read guarantee.
    fn get_canonical_tip_blocking(&self) -> DbResult<Option<OLBlockCommitment>> {
        self.blocks.get_canonical_tip_blocking()
    }

    /// Returns `None` when no stored commitment for `epoch` matches the canonical index.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if multiple terminals for the epoch match the canonical index.
    async fn get_canonical_epoch_commitment_at_async(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        let candidates = self
            .checkpoints
            .get_epoch_commitments_at_async(epoch)
            .await?;
        if candidates.is_empty() {
            return Ok(None);
        }
        let slots = candidates.iter().map(EpochCommitment::last_slot).collect();
        let canonical_ids = self.blocks.get_canonical_blocks_async(slots).await?;
        Ok(select_matching(candidates, canonical_ids))
    }

    /// Returns `None` when no stored commitment for `epoch` matches the canonical index.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if multiple terminals for the epoch match the canonical index.
    fn get_canonical_epoch_commitment_at_blocking(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        let candidates = self.checkpoints.get_epoch_commitments_at_blocking(epoch)?;
        if candidates.is_empty() {
            return Ok(None);
        }
        let slots = candidates.iter().map(EpochCommitment::last_slot).collect();
        let canonical_ids = self.blocks.get_canonical_blocks_blocking(slots)?;
        Ok(select_matching(candidates, canonical_ids))
    }
}

fn select_matching(
    candidates: Vec<EpochCommitment>,
    canonical_ids: Vec<Option<OLBlockId>>,
) -> Option<EpochCommitment> {
    let mut matches = candidates
        .into_iter()
        .zip(canonical_ids)
        .filter(|(candidate, id)| id.as_ref() == Some(candidate.last_blkid()))
        .map(|(candidate, _)| candidate);
    let selected = matches.next();
    debug_assert!(
        matches.next().is_none(),
        "multiple terminals for one epoch match the canonical index"
    );
    selected
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_checkpoint_types::EpochSummary;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{
        Buf32, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
        Slot,
    };
    use strata_storage::{OLBlockManager, OLCheckpointManager, create_node_storage};
    use tokio::runtime::Runtime;

    use super::OLCanonicalChainProviderImpl;
    use crate::OLCanonicalChainProvider;

    struct TestContext {
        blocks: Arc<OLBlockManager>,
        checkpoints: Arc<OLCheckpointManager>,
        provider: OLCanonicalChainProviderImpl,
        runtime: Runtime,
    }

    impl TestContext {
        fn new() -> Self {
            let runtime = Runtime::new().unwrap();
            let db = get_test_sled_backend();
            let storage = create_node_storage(db, runtime.handle().clone())
                .expect("test: create node storage");
            let blocks = storage.ol_block().clone();
            let checkpoints = storage.ol_checkpoint().clone();
            let provider = OLCanonicalChainProviderImpl::new(&storage);
            Self {
                blocks,
                checkpoints,
                provider,
                runtime,
            }
        }

        fn insert_candidate(&self, candidate: EpochCommitment) {
            // Only candidate metadata is needed; no block bodies or headers are inserted.
            let summary = EpochSummary::new(
                candidate.epoch(),
                candidate.to_block_commitment(),
                OLBlockCommitment::new(0, block_id(0x10)),
                L1BlockCommitment::new(100, L1BlockId::from(Buf32::from([0x20; 32]))),
                Buf32::from([0x30; 32]),
            );
            self.checkpoints
                .insert_epoch_summary_blocking(summary)
                .expect("test: insert candidate summary");
        }

        fn replace_suffix(&self, slot: Slot, ids: Vec<OLBlockId>) {
            self.blocks
                .replace_canonical_suffix_from_blocking(slot, ids)
                .expect("test: replace canonical suffix");
        }

        fn assert_epoch(&self, epoch: Epoch, expected: Option<EpochCommitment>) {
            assert_eq!(
                self.provider
                    .get_canonical_epoch_commitment_at_blocking(epoch)
                    .unwrap(),
                expected
            );
            assert_eq!(
                self.runtime
                    .block_on(self.provider.get_canonical_epoch_commitment_at_async(epoch))
                    .unwrap(),
                expected
            );
        }

        fn assert_block(&self, slot: Slot, expected: Option<OLBlockCommitment>) {
            assert_eq!(
                self.provider.get_canonical_block_at_blocking(slot).unwrap(),
                expected
            );
            assert_eq!(
                self.runtime
                    .block_on(self.provider.get_canonical_block_at_async(slot))
                    .unwrap(),
                expected
            );
        }

        fn assert_tip(&self, expected: Option<OLBlockCommitment>) {
            assert_eq!(
                self.provider.get_canonical_tip_blocking().unwrap(),
                expected
            );
            assert_eq!(
                self.runtime
                    .block_on(self.provider.get_canonical_tip_async())
                    .unwrap(),
                expected
            );
        }
    }

    fn block_id(byte: u8) -> OLBlockId {
        OLBlockId::from(Buf32::from([byte; 32]))
    }

    #[test]
    fn test_empty_storage_returns_none() {
        let context = TestContext::new();
        context.assert_block(5, None);
        context.assert_tip(None);
        context.assert_epoch(1, None);
    }

    #[test]
    fn test_block_and_tip_follow_sparse_index_and_reorg() {
        let context = TestContext::new();
        let first = OLBlockCommitment::new(5, block_id(0xa1));
        let second = OLBlockCommitment::new(9, block_id(0xa2));
        context.replace_suffix(first.slot(), vec![*first.blkid()]);
        context.replace_suffix(second.slot(), vec![*second.blkid()]);
        context.assert_block(5, Some(first));
        context.assert_block(7, None);
        context.assert_block(9, Some(second));
        context.assert_tip(Some(second));

        // Reorg to a shorter branch, then remove it; reads must not retain the old tip or rows.
        let replacement = OLBlockCommitment::new(5, block_id(0xb1));
        context.replace_suffix(5, vec![*replacement.blkid()]);
        context.assert_block(5, Some(replacement));
        context.assert_block(9, None);
        context.assert_tip(Some(replacement));
        context.replace_suffix(5, vec![]);
        context.assert_tip(None);
    }

    #[test]
    fn test_same_slot_candidates_select_canonical_id() {
        let candidates = [
            EpochCommitment::new(1, 5, block_id(0xa1)),
            EpochCommitment::new(1, 5, block_id(0xb1)),
        ];
        for insertion_order in [candidates, [candidates[1], candidates[0]]] {
            let context = TestContext::new();
            for candidate in insertion_order {
                context.insert_candidate(candidate);
            }

            // Choose the candidate that sorts last, then reorg to the other one at the same slot.
            for canonical in [candidates[1], candidates[0]] {
                context.replace_suffix(5, vec![*canonical.last_blkid()]);
                context.assert_epoch(1, Some(canonical));
            }
        }
    }

    #[test]
    fn test_different_slot_candidates_match_sparse_canonical_rows() {
        let context = TestContext::new();
        let early_fork = EpochCommitment::new(1, 5, block_id(0xa1));
        let same_slot_fork = EpochCommitment::new(1, 5, block_id(0xa4));
        let canonical = EpochCommitment::new(1, 9, block_id(0xa2));
        let late_fork = EpochCommitment::new(1, 12, block_id(0xa3));
        for candidate in [late_fork, canonical, early_fork, same_slot_fork] {
            context.insert_candidate(candidate);
        }

        // The early terminal's slot is canonical with a different ID; the late slot is absent.
        context.replace_suffix(5, vec![block_id(0xb1)]);
        context.replace_suffix(9, vec![*canonical.last_blkid()]);
        context.assert_epoch(1, Some(canonical));
        context.assert_epoch(0, None);
        context.assert_epoch(2, None);

        // Keep all candidates while replacing the canonical branch with a different terminal slot.
        context.replace_suffix(5, vec![]);
        context.replace_suffix(12, vec![*late_fork.last_blkid()]);
        context.assert_epoch(1, Some(late_fork));
    }

    #[test]
    fn test_candidates_without_canonical_match_return_none() {
        let context = TestContext::new();
        let candidate = EpochCommitment::new(1, 5, block_id(0xa1));
        context.insert_candidate(candidate);
        context.assert_epoch(1, None);

        // A canonical row at the right slot is insufficient when the ID differs.
        context.replace_suffix(5, vec![block_id(0xb1)]);
        context.assert_epoch(1, None);
    }

    #[test]
    fn test_canonical_row_requires_candidate_for_requested_epoch() {
        let context = TestContext::new();
        let candidate = EpochCommitment::new(1, 5, block_id(0xa1));
        context.replace_suffix(5, vec![*candidate.last_blkid()]);
        context.assert_epoch(1, None);

        context.insert_candidate(candidate);
        context.assert_epoch(1, Some(candidate));
        context.assert_epoch(2, None);

        // Candidate removal must be reflected even while its canonical row remains.
        context
            .checkpoints
            .del_epoch_summary_blocking(candidate)
            .expect("test: remove candidate summary");
        context.assert_epoch(1, None);
        context.assert_block(5, Some(candidate.to_block_commitment()));
    }

    #[test]
    fn test_cloned_provider_observes_storage_updates() {
        let context = TestContext::new();
        let cloned = context.provider.clone();
        let candidate = EpochCommitment::new(1, 5, block_id(0xa1));
        context.insert_candidate(candidate);
        context.replace_suffix(5, vec![*candidate.last_blkid()]);
        assert_eq!(
            cloned
                .get_canonical_epoch_commitment_at_blocking(1)
                .unwrap(),
            Some(candidate)
        );

        context.replace_suffix(5, vec![]);
        assert_eq!(
            context
                .runtime
                .block_on(cloned.get_canonical_epoch_commitment_at_async(1))
                .unwrap(),
            None
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "multiple terminals for one epoch match the canonical index")]
    fn test_multiple_canonical_terminals_trigger_debug_assertion() {
        let context = TestContext::new();
        let first = EpochCommitment::new(1, 5, block_id(0xa1));
        let second = EpochCommitment::new(1, 9, block_id(0xa2));
        context.insert_candidate(first);
        context.insert_candidate(second);

        // Inject two indexed terminals for the same epoch, violating the canonical-chain invariant.
        context.replace_suffix(5, vec![*first.last_blkid()]);
        context.replace_suffix(9, vec![*second.last_blkid()]);
        context
            .provider
            .get_canonical_epoch_commitment_at_blocking(1)
            .unwrap();
    }
}
