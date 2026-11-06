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
    ol_block: Arc<OLBlockManager>,
    ol_checkpoint: Arc<OLCheckpointManager>,
}

impl OLCanonicalChainProviderImpl {
    /// Creates a provider using the block and checkpoint managers from `storage`.
    pub fn new(storage: &NodeStorage) -> Self {
        Self {
            ol_block: storage.ol_block().clone(),
            ol_checkpoint: storage.ol_checkpoint().clone(),
        }
    }
}

impl OLCanonicalChainProvider for OLCanonicalChainProviderImpl {
    /// Returns `None` when the canonical index has no entry at `slot`.
    async fn get_canonical_block_at_async(
        &self,
        slot: Slot,
    ) -> DbResult<Option<OLBlockCommitment>> {
        self.ol_block.get_canonical_block_at_async(slot).await
    }

    fn get_canonical_block_at_blocking(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>> {
        self.ol_block.get_canonical_block_at_blocking(slot)
    }

    /// Returns `None` when the canonical index is empty.
    ///
    /// The slot and block ID are read together, even during a concurrent canonical
    /// suffix replacement.
    async fn get_canonical_tip_async(&self) -> DbResult<Option<OLBlockCommitment>> {
        self.ol_block.get_canonical_tip_async().await
    }

    /// Blocking counterpart of [`Self::get_canonical_tip_async`], with the same read guarantee.
    fn get_canonical_tip_blocking(&self) -> DbResult<Option<OLBlockCommitment>> {
        self.ol_block.get_canonical_tip_blocking()
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
            .ol_checkpoint
            .get_epoch_commitments_at_async(epoch)
            .await?;
        if candidates.is_empty() {
            return Ok(None);
        }
        let slots = candidates.iter().map(EpochCommitment::last_slot).collect();
        let canonical_ids = self.ol_block.get_canonical_blocks_async(slots).await?;
        Ok(select_canonical_epoch_commitment(candidates, canonical_ids))
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
        let candidates = self
            .ol_checkpoint
            .get_epoch_commitments_at_blocking(epoch)?;
        if candidates.is_empty() {
            return Ok(None);
        }
        let slots = candidates.iter().map(EpochCommitment::last_slot).collect();
        let canonical_ids = self.ol_block.get_canonical_blocks_blocking(slots)?;
        Ok(select_canonical_epoch_commitment(candidates, canonical_ids))
    }
}

/// Returns the stored commitment whose last block is the canonical block at its slot,
/// or `None` if no stored commitment for the epoch is on the canonical chain.
///
/// `canonical_ids[i]` is the canonical block at `candidates[i]`'s last slot.
fn select_canonical_epoch_commitment(
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
        ol_block: Arc<OLBlockManager>,
        ol_checkpoint: Arc<OLCheckpointManager>,
        provider: OLCanonicalChainProviderImpl,
        runtime: Runtime,
    }

    impl TestContext {
        fn new() -> Self {
            let runtime = Runtime::new().unwrap();
            let db = get_test_sled_backend();
            let storage = create_node_storage(db, runtime.handle().clone())
                .expect("test: create node storage");
            let ol_block = storage.ol_block().clone();
            let ol_checkpoint = storage.ol_checkpoint().clone();
            let provider = OLCanonicalChainProviderImpl::new(&storage);
            Self {
                ol_block,
                ol_checkpoint,
                provider,
                runtime,
            }
        }

        fn insert_candidate(&self, candidate: EpochCommitment) {
            // Only candidate metadata is needed; no block bodies or headers are inserted.
            let summary = EpochSummary::new(
                candidate.epoch(),
                candidate.to_block_commitment(),
                OLBlockCommitment::new(0, make_block_id(0x10)),
                L1BlockCommitment::new(100, L1BlockId::from(Buf32::from([0x20; 32]))),
                Buf32::from([0x30; 32]),
            );
            self.ol_checkpoint
                .insert_epoch_summary_blocking(summary)
                .expect("test: insert candidate summary");
        }

        fn replace_suffix(&self, slot: Slot, ids: Vec<OLBlockId>) {
            self.ol_block
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

    fn make_block_id(byte: u8) -> OLBlockId {
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
        let first = OLBlockCommitment::new(5, make_block_id(0xa1));
        let second = OLBlockCommitment::new(9, make_block_id(0xa2));
        context.replace_suffix(first.slot(), vec![*first.blkid()]);
        context.replace_suffix(second.slot(), vec![*second.blkid()]);
        context.assert_block(5, Some(first));
        context.assert_block(7, None);
        context.assert_block(9, Some(second));
        context.assert_tip(Some(second));

        // Reorg to a shorter branch, then remove it; reads must not retain the old tip or rows.
        let replacement = OLBlockCommitment::new(5, make_block_id(0xb1));
        context.replace_suffix(5, vec![*replacement.blkid()]);
        context.assert_block(5, Some(replacement));
        context.assert_block(9, None);
        context.assert_tip(Some(replacement));
        context.replace_suffix(5, vec![]);
        context.assert_tip(None);
    }

    #[test]
    fn test_same_slot_candidates_select_canonical_id() {
        // Two branches end epoch 1 at the same slot with different blocks. The selection must
        // follow the canonical index, not storage order: the replaced lookup returned the first
        // stored candidate. Both insertion orders are tried.
        let candidates = [
            EpochCommitment::new(1, 5, make_block_id(0xa1)),
            EpochCommitment::new(1, 5, make_block_id(0xb1)),
        ];
        for insertion_order in [candidates, [candidates[1], candidates[0]]] {
            let context = TestContext::new();
            for candidate in insertion_order {
                context.insert_candidate(candidate);
            }

            // Index the candidate that sorts last, then reorg the slot to the other one.
            for canonical in [candidates[1], candidates[0]] {
                context.replace_suffix(5, vec![*canonical.last_blkid()]);
                context.assert_epoch(1, Some(canonical));
            }
        }
    }

    #[test]
    fn test_different_slot_candidates_match_sparse_canonical_rows() {
        // Branches can end epoch 1 at different slots. Each candidate is checked against the
        // canonical block at its own last slot.
        let context = TestContext::new();
        let early_fork = EpochCommitment::new(1, 5, make_block_id(0xa1));
        let same_slot_fork = EpochCommitment::new(1, 5, make_block_id(0xa4));
        let canonical = EpochCommitment::new(1, 9, make_block_id(0xa2));
        let late_fork = EpochCommitment::new(1, 12, make_block_id(0xa3));
        for candidate in [late_fork, canonical, early_fork, same_slot_fork] {
            context.insert_candidate(candidate);
        }

        // Slot 5 holds a different canonical block and slot 12 has none, so only the
        // candidate ending at slot 9 is canonical.
        context.replace_suffix(5, vec![make_block_id(0xb1)]);
        context.replace_suffix(9, vec![*canonical.last_blkid()]);
        context.assert_epoch(1, Some(canonical));
        context.assert_epoch(0, None);
        context.assert_epoch(2, None);

        // Reorg to the branch whose epoch 1 ends at slot 12, keeping all stored candidates.
        context.replace_suffix(5, vec![]);
        context.replace_suffix(12, vec![*late_fork.last_blkid()]);
        context.assert_epoch(1, Some(late_fork));
    }

    #[test]
    fn test_candidates_without_canonical_match_return_none() {
        let context = TestContext::new();
        let candidate = EpochCommitment::new(1, 5, make_block_id(0xa1));
        context.insert_candidate(candidate);
        context.assert_epoch(1, None);

        // A canonical row at the right slot is insufficient when the ID differs.
        context.replace_suffix(5, vec![make_block_id(0xb1)]);
        context.assert_epoch(1, None);
    }

    #[test]
    fn test_canonical_row_requires_candidate_for_requested_epoch() {
        let context = TestContext::new();
        let candidate = EpochCommitment::new(1, 5, make_block_id(0xa1));
        // A canonical block alone does not make an epoch: its commitment must also be stored.
        context.replace_suffix(5, vec![*candidate.last_blkid()]);
        context.assert_epoch(1, None);

        context.insert_candidate(candidate);
        context.assert_epoch(1, Some(candidate));
        context.assert_epoch(2, None);

        // Candidate removal must be reflected even while its canonical row remains.
        context
            .ol_checkpoint
            .del_epoch_summary_blocking(candidate)
            .expect("test: remove candidate summary");
        context.assert_epoch(1, None);
        context.assert_block(5, Some(candidate.to_block_commitment()));
    }

    #[test]
    fn test_cloned_provider_observes_storage_updates() {
        // Clones share the storage managers, so they read current storage, not a copy.
        let context = TestContext::new();
        let cloned = context.provider.clone();
        let candidate = EpochCommitment::new(1, 5, make_block_id(0xa1));
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
        let first = EpochCommitment::new(1, 5, make_block_id(0xa1));
        let second = EpochCommitment::new(1, 9, make_block_id(0xa2));
        context.insert_candidate(first);
        context.insert_candidate(second);

        // Index both candidates' last blocks, so two commitments for one epoch are canonical.
        // A canonical chain has one end per epoch, so this is an invariant violation.
        context.replace_suffix(5, vec![*first.last_blkid()]);
        context.replace_suffix(9, vec![*second.last_blkid()]);
        context
            .provider
            .get_canonical_epoch_commitment_at_blocking(1)
            .unwrap();
    }
}
