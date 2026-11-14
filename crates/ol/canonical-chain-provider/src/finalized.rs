use std::sync::Arc;

use strata_db_types::{DbError, DbResult};
use strata_identifiers::{Epoch, EpochCommitment, OLBlockCommitment, Slot};
use strata_storage::{ClientStateManager, L1BlockManager, NodeStorage, OLCheckpointManager};

use tokio::task::spawn_blocking;

use crate::{OLCanonicalChainProvider, resolve_observed_epoch_blocking};

/// Resolves the finalized OL view from client state and canonical L1 checkpoint observations.
///
/// The view ends at the latest finalized epoch declared by client state. Each query reads
/// that epoch once and answers against it, except a query for epoch zero, which resolves
/// from local genesis metadata without reading L1 finality. Other epochs before the
/// finalized one resolve from their unique checkpoint observation on canonical L1;
/// missing or ambiguous history is an error. Slot queries expose only the last block of
/// each finalized epoch. Local checkpoint application and the OL canonical index are not
/// used.
///
/// Its reads of the client-state, checkpoint, and L1 stores are not one atomic snapshot.
/// If the latest client-state row's L1 block is no longer canonical, calls fail with
/// [`DbError::OLFinalizedClientStateUnavailable`], and the caller can retry once the
/// client-state tracker has caught up.
#[derive(Clone)]
#[expect(
    missing_debug_implementations,
    reason = "storage managers do not implement Debug"
)]
pub struct FinalizedOLChainProvider {
    client_state: Arc<ClientStateManager>,
    ol_checkpoint: Arc<OLCheckpointManager>,
    l1: Arc<L1BlockManager>,
}

impl FinalizedOLChainProvider {
    /// Creates a provider using managers from the same node storage.
    pub fn new(storage: &NodeStorage) -> Self {
        Self {
            client_state: storage.client_state().clone(),
            ol_checkpoint: storage.ol_checkpoint().clone(),
            l1: storage.l1().clone(),
        }
    }

    /// Returns the genesis epoch commitment, or `None` if it is not stored.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::OLFinalizedHistoryUnavailable`] if the stored commitment is not a
    /// valid genesis (slot 0, non-null), and [`DbError::OLCheckpointHistoryAmbiguous`] if
    /// several are stored.
    fn resolve_genesis_blocking(&self) -> DbResult<Option<EpochCommitment>> {
        let candidates = self.ol_checkpoint.get_epoch_commitments_at_blocking(0)?;
        match candidates.as_slice() {
            [] => Ok(None),
            [genesis] if genesis.last_slot() == 0 && *genesis != EpochCommitment::null() => {
                Ok(Some(*genesis))
            }
            [_] => Err(DbError::OLFinalizedHistoryUnavailable { epoch: 0 }),
            _ => Err(DbError::OLCheckpointHistoryAmbiguous { epoch: 0 }),
        }
    }

    /// Returns the latest finalized epoch declared by the latest client-state row, or the
    /// genesis epoch before a nonzero epoch is finalized. Returns `None` if neither exists.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::OLFinalizedClientStateUnavailable`] if the latest client-state
    /// row's L1 block is no longer on the canonical L1 chain.
    fn read_finalized_epoch_blocking(&self) -> DbResult<Option<EpochCommitment>> {
        if let Some((block, state)) = self.client_state.fetch_most_recent_state()? {
            if self.l1.get_canonical_blockid_at_height(block.height())? != Some(*block.blkid()) {
                return Err(DbError::OLFinalizedClientStateUnavailable { block });
            }
            if let Some(finalized) = state.get_declared_final_epoch().filter(|c| c.epoch() > 0) {
                return Ok(Some(finalized));
            }
        }
        self.resolve_genesis_blocking()
    }

    /// Returns the commitment at `epoch`, which must not be after `finalized`.
    ///
    /// Returns `finalized` itself at its epoch. Earlier, epoch 0 resolves from genesis and
    /// later epochs from their unique observed checkpoint on canonical L1, which must end
    /// before `finalized`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::OLFinalizedHistoryUnavailable`] or
    /// [`DbError::OLCheckpointHistoryAmbiguous`] if that history is missing or ambiguous,
    /// and [`DbError::OLFinalizedHistoryOrder`] if it does not end before `finalized`.
    fn resolve_epoch_up_to_blocking(
        &self,
        finalized: EpochCommitment,
        epoch: Epoch,
    ) -> DbResult<EpochCommitment> {
        if epoch == finalized.epoch() {
            return Ok(finalized);
        }
        if epoch == 0 {
            return self
                .resolve_genesis_blocking()?
                .ok_or(DbError::OLFinalizedHistoryUnavailable { epoch });
        }
        let selected = resolve_observed_epoch_blocking(&self.ol_checkpoint, &self.l1, epoch)?
            .ok_or(DbError::OLFinalizedHistoryUnavailable { epoch })?;
        check_order(selected, finalized)?;
        Ok(selected)
    }
}

impl OLCanonicalChainProvider for FinalizedOLChainProvider {
    async fn get_canonical_tip_async(&self) -> DbResult<Option<OLBlockCommitment>> {
        let provider = self.clone();
        spawn_blocking(move || provider.get_canonical_tip_blocking()).await?
    }

    async fn get_canonical_epoch_commitment_at_async(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        let provider = self.clone();
        spawn_blocking(move || provider.get_canonical_epoch_commitment_at_blocking(epoch)).await?
    }

    async fn get_canonical_block_at_async(
        &self,
        slot: Slot,
    ) -> DbResult<Option<OLBlockCommitment>> {
        let provider = self.clone();
        spawn_blocking(move || provider.get_canonical_block_at_blocking(slot)).await?
    }

    fn get_canonical_tip_blocking(&self) -> DbResult<Option<OLBlockCommitment>> {
        Ok(self
            .read_finalized_epoch_blocking()?
            .map(|finalized| finalized.to_block_commitment()))
    }

    fn get_canonical_epoch_commitment_at_blocking(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        // Genesis does not depend on L1 finality. A missing genesis falls through to the
        // finalized-epoch lookup, which reports the unavailable history once an epoch is
        // finalized.
        if epoch == 0
            && let Some(genesis) = self.resolve_genesis_blocking()?
        {
            return Ok(Some(genesis));
        }
        let Some(finalized) = self.read_finalized_epoch_blocking()? else {
            return Ok(None);
        };
        if epoch > finalized.epoch() {
            return Ok(None);
        }
        self.resolve_epoch_up_to_blocking(finalized, epoch)
            .map(Some)
    }

    fn get_canonical_block_at_blocking(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>> {
        let Some(finalized) = self.read_finalized_epoch_blocking()? else {
            return Ok(None);
        };
        if slot > finalized.last_slot() {
            return Ok(None);
        }
        if slot == finalized.last_slot() {
            return Ok(Some(finalized.to_block_commitment()));
        }
        if finalized.epoch() == 0 {
            return Ok(None);
        }
        // Only the last block of each finalized epoch is known; look up which epoch, if
        // any, ends at `slot`.
        let genesis = self.resolve_epoch_up_to_blocking(finalized, 0)?;
        let mut bounds = EpochEndBounds::new(slot, genesis, finalized)?;
        loop {
            if let Some(result) = bounds.result() {
                return Ok(result);
            }
            let candidate = self.resolve_epoch_up_to_blocking(finalized, bounds.next_epoch())?;
            bounds.advance(candidate)?;
        }
    }
}

/// Search state for finding the finalized epoch that ends at a slot.
///
/// The finalized view knows only the last block of each finalized epoch (its terminal
/// block, which the epoch's checkpoint commits to). Epoch numbers are contiguous, but
/// epochs end at irregular slots, so a slot cannot be mapped to an epoch directly. The
/// caller looks up the epoch [`Self::next_epoch`] names and passes it to
/// [`Self::advance`], halving the range each time, until [`Self::result`] is known. This
/// type only holds and narrows the bounds; it reads no storage. A failed lookup is the
/// caller's error, not evidence that no epoch ends at the slot.
struct EpochEndBounds {
    /// Slot being looked up.
    slot: Slot,

    /// Nearest known epoch ending at or before `slot`.
    below: EpochCommitment,

    /// Nearest known epoch ending after `slot`.
    above: EpochCommitment,
}

impl EpochEndBounds {
    /// Starts the search between two known epochs, `below` ending before `above`.
    fn new(slot: Slot, below: EpochCommitment, above: EpochCommitment) -> DbResult<Self> {
        check_order(below, above)?;
        Ok(Self { slot, below, above })
    }

    /// Returns the outcome once known: `Some(Some(block))` if an epoch ends at the slot,
    /// `Some(None)` if none does, and `None` while the search must continue.
    fn result(&self) -> Option<Option<OLBlockCommitment>> {
        if self.slot == self.below.last_slot() {
            Some(Some(self.below.to_block_commitment()))
        } else if self.slot == self.above.last_slot() {
            Some(Some(self.above.to_block_commitment()))
        } else if self.slot < self.below.last_slot() || self.above.epoch() - self.below.epoch() == 1
        {
            Some(None)
        } else {
            None
        }
    }

    /// Returns the epoch halfway between the bounds, to look up next.
    fn next_epoch(&self) -> Epoch {
        self.below.epoch() + (self.above.epoch() - self.below.epoch()) / 2
    }

    /// Narrows the bounds with the looked-up epoch `candidate`, which must end strictly
    /// between them.
    fn advance(&mut self, candidate: EpochCommitment) -> DbResult<()> {
        check_order(self.below, candidate)?;
        check_order(candidate, self.above)?;
        if candidate.last_slot() <= self.slot {
            self.below = candidate;
        } else {
            self.above = candidate;
        }
        Ok(())
    }
}

fn check_order(earlier: EpochCommitment, later: EpochCommitment) -> DbResult<()> {
    if earlier.last_slot() >= later.last_slot() {
        return Err(DbError::OLFinalizedHistoryOrder { earlier, later });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use strata_checkpoint_types::EpochSummary;
    use strata_csm_types::{ClientState, ClientUpdateOutput, L1Checkpoint};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::client_state::ClientStateDatabase;
    use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId};
    use strata_storage::create_node_storage;
    use tokio::runtime::Runtime;

    use super::*;
    use crate::test_utils::{build_checkpoint_payload, finalize_epoch, record_l1_observation};

    struct Fixture {
        storage: NodeStorage,
        provider: FinalizedOLChainProvider,
        runtime: Runtime,
        genesis: EpochCommitment,
    }

    impl Fixture {
        fn empty() -> Self {
            let runtime = Runtime::new().unwrap();
            let storage =
                create_node_storage(get_test_sled_backend(), runtime.handle().clone()).unwrap();
            Self {
                provider: FinalizedOLChainProvider::new(&storage),
                storage,
                runtime,
                genesis: EpochCommitment::new(0, 0, Buf32::from([1; 32]).into()),
            }
        }

        fn new() -> Self {
            let fixture = Self::empty();
            // Genesis is initialized locally; later epochs will be observed from L1 only.
            fixture
                .storage
                .ol_checkpoint()
                .insert_epoch_summary_blocking(EpochSummary::new(
                    0,
                    fixture.genesis.to_block_commitment(),
                    OLBlockCommitment::null(),
                    L1BlockCommitment::new(100, Buf32::from([100; 32]).into()),
                    Buf32::from([1; 32]),
                ))
                .unwrap();
            fixture
                .storage
                .ol_block()
                .replace_canonical_suffix_from_blocking(0, vec![*fixture.genesis.last_blkid()])
                .unwrap();
            fixture.advance_l1();
            fixture
        }

        fn advance_l1(&self) -> L1BlockCommitment {
            let height = self
                .storage
                .l1()
                .get_chain_tip_height()
                .unwrap()
                .map_or(100, |height| height + 1);
            let block = L1BlockCommitment::new(height, Buf32::from([height as u8; 32]).into());
            self.storage
                .l1()
                .extend_canonical_chain(block.blkid(), height)
                .unwrap();
            block
        }

        fn observe_after(&self, previous: EpochCommitment, seed: u8) -> EpochCommitment {
            let commitment = EpochCommitment::new(
                previous.epoch() + 1,
                previous.last_slot() + 3,
                Buf32::from([seed; 32]).into(),
            );
            self.store_observation(commitment, self.advance_l1());
            commitment
        }

        fn store_observation(&self, commitment: EpochCommitment, block: L1BlockCommitment) {
            // Only the observation path is populated, not the sequencer payload/signing tables.
            record_l1_observation(
                &self.storage,
                commitment,
                build_checkpoint_payload(commitment, block.height()),
                block,
            );
        }

        fn finalize(&self, commitment: EpochCommitment) -> L1BlockCommitment {
            let checkpoints = self.storage.ol_checkpoint();
            let payload = checkpoints
                .get_checkpoint_l1_observed_payload_blocking(commitment)
                .unwrap()
                .unwrap();
            let reference = checkpoints
                .get_checkpoint_l1_ref_blocking(commitment)
                .unwrap()
                .unwrap();
            let block = self.advance_l1();
            finalize_epoch(
                &self.storage,
                L1Checkpoint::new(*payload.new_tip(), reference),
                block,
            );
            block
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

        fn assert_tip(&self, expected: Option<EpochCommitment>) {
            let expected = expected.map(|c| c.to_block_commitment());
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

        fn assert_slot(&self, slot: Slot, expected: Option<EpochCommitment>) {
            let expected = expected.map(|c| c.to_block_commitment());
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

        fn epoch_errors(&self, epoch: Epoch) -> [DbError; 2] {
            [
                self.provider
                    .get_canonical_epoch_commitment_at_blocking(epoch)
                    .unwrap_err(),
                self.runtime
                    .block_on(self.provider.get_canonical_epoch_commitment_at_async(epoch))
                    .unwrap_err(),
            ]
        }
    }

    #[test]
    fn test_empty_and_genesis_views() {
        let fixture = Fixture::empty();
        fixture.assert_tip(None);
        fixture.assert_epoch(0, None);
        fixture.assert_slot(0, None);
        let fixture = Fixture::new();
        fixture.assert_tip(Some(fixture.genesis));
        fixture.assert_epoch(0, Some(fixture.genesis));
        fixture.assert_epoch(1, None);
        fixture.assert_slot(0, Some(fixture.genesis));
        fixture.assert_slot(1, None);
    }

    #[test]
    fn test_finality_advances_without_application_or_sequencer_artifacts() {
        let fixture = Fixture::new();
        let first = fixture.observe_after(fixture.genesis, 2);
        let second = fixture.observe_after(first, 3);
        fixture.assert_epoch(first.epoch(), None);
        fixture.finalize(first);
        fixture.assert_tip(Some(first));
        fixture.assert_epoch(first.epoch(), Some(first));
        fixture.assert_slot(first.last_slot(), Some(first));
        fixture.assert_epoch(second.epoch(), None);

        // Applied metadata cannot lift the finalized bound either.
        fixture
            .storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(EpochSummary::new(
                second.epoch(),
                second.to_block_commitment(),
                first.to_block_commitment(),
                L1BlockCommitment::new(102, Buf32::from([102; 32]).into()),
                Buf32::from([3; 32]),
            ))
            .unwrap();
        fixture.assert_epoch(second.epoch(), None);
        fixture.finalize(second);
        fixture.assert_tip(Some(second));
        fixture.assert_epoch(first.epoch(), Some(first));
        fixture.assert_epoch(second.epoch(), Some(second));
        fixture.assert_slot(second.last_slot(), Some(second));

        assert_eq!(
            fixture
                .storage
                .ol_checkpoint()
                .get_epoch_summary_blocking(first)
                .unwrap(),
            None
        );
        assert_eq!(
            fixture
                .storage
                .ol_checkpoint()
                .get_checkpoint_payload_entry_blocking(first)
                .unwrap(),
            None
        );
        assert_eq!(
            fixture
                .storage
                .ol_checkpoint()
                .get_checkpoint_signing_entry_blocking(first)
                .unwrap(),
            None
        );
        assert!(
            fixture
                .storage
                .ol_block()
                .get_ol_header_blocking(*first.last_blkid())
                .unwrap()
                .is_none()
        );
        assert!(
            fixture
                .storage
                .ol_state()
                .get_toplevel_ol_state_blocking(first.to_block_commitment())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            fixture
                .storage
                .ol_block()
                .get_canonical_tip_blocking()
                .unwrap(),
            Some(fixture.genesis.to_block_commitment())
        );

        // Reopening managers recovers the same finality bound from persisted client state.
        let restarted = create_node_storage(
            fixture.storage.db().clone(),
            fixture.runtime.handle().clone(),
        )
        .unwrap();
        let provider = FinalizedOLChainProvider::new(&restarted);
        assert_eq!(
            provider.get_canonical_tip_blocking().unwrap(),
            Some(second.to_block_commitment())
        );
        assert_eq!(
            fixture
                .runtime
                .block_on(provider.get_canonical_epoch_commitment_at_async(first.epoch()))
                .unwrap(),
            Some(first)
        );
    }

    #[test]
    fn test_finalized_tip_does_not_require_observation_or_genesis_history() {
        let fixture = Fixture::new();
        let first = fixture.observe_after(fixture.genesis, 2);
        fixture.finalize(first);
        fixture
            .storage
            .ol_checkpoint()
            .del_checkpoint_l1_ref_blocking(first)
            .unwrap();
        fixture
            .storage
            .ol_checkpoint()
            .del_epoch_summary_blocking(fixture.genesis)
            .unwrap();
        fixture.assert_tip(Some(first));
        fixture.assert_epoch(first.epoch(), Some(first));
        fixture.assert_slot(first.last_slot(), Some(first));
        for error in fixture.epoch_errors(0) {
            assert!(matches!(
                error,
                DbError::OLFinalizedHistoryUnavailable { epoch: 0 }
            ));
        }
    }

    #[test]
    fn test_terminal_slot_search_preserves_sparse_gaps() {
        let fixture = Fixture::new();
        let first = fixture.observe_after(fixture.genesis, 2);
        let second = fixture.observe_after(first, 3);
        let third = fixture.observe_after(second, 4);
        fixture.finalize(third);
        for slot in 0..=third.last_slot() + 1 {
            let expected = [fixture.genesis, first, second, third]
                .into_iter()
                .find(|c| c.last_slot() == slot);
            fixture.assert_slot(slot, expected);
        }
        fixture
            .storage
            .ol_checkpoint()
            .del_checkpoint_l1_ref_blocking(first)
            .unwrap();
        for error in [
            fixture
                .provider
                .get_canonical_block_at_blocking(1)
                .unwrap_err(),
            fixture
                .runtime
                .block_on(fixture.provider.get_canonical_block_at_async(1))
                .unwrap_err(),
        ] {
            assert!(matches!(
                error,
                DbError::OLFinalizedHistoryUnavailable { epoch: 1 }
            ));
        }
    }

    #[test]
    fn test_orphan_observations_filtered_and_multiple_survivors_rejected() {
        let fixture = Fixture::new();
        let first = fixture.observe_after(fixture.genesis, 2);
        let first_ref = fixture
            .storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_blocking(first)
            .unwrap()
            .unwrap();
        let replacement = EpochCommitment::new(
            first.epoch(),
            first.last_slot(),
            Buf32::from([3; 32]).into(),
        );
        // Replace the observation's L1 block while retaining both checkpoint references.
        fixture
            .storage
            .l1()
            .revert_canonical_chain(first_ref.block_height() - 1)
            .unwrap();
        let replacement_block = L1BlockCommitment::new(
            first_ref.block_height(),
            L1BlockId::from(Buf32::from([99; 32])),
        );
        fixture
            .storage
            .l1()
            .extend_canonical_chain(replacement_block.blkid(), replacement_block.height())
            .unwrap();
        assert_eq!(
            resolve_observed_epoch_blocking(
                fixture.storage.ol_checkpoint(),
                fixture.storage.l1(),
                first.epoch()
            )
            .unwrap(),
            None,
        );
        fixture.store_observation(replacement, replacement_block);
        let second = fixture.observe_after(replacement, 4);
        fixture.finalize(second);
        fixture.assert_epoch(1, Some(replacement));

        // Two different commitments on canonical L1 are inconsistent, not two choices.
        fixture.store_observation(first, replacement_block);
        for error in fixture.epoch_errors(1) {
            assert!(matches!(
                error,
                DbError::OLCheckpointHistoryAmbiguous { epoch: 1 }
            ));
        }
    }

    #[test]
    fn test_orphaned_client_state_rejected() {
        let fixture = Fixture::new();
        let first = fixture.observe_after(fixture.genesis, 2);
        let state_block = fixture.finalize(first);
        fixture
            .storage
            .l1()
            .revert_canonical_chain(state_block.height() - 1)
            .unwrap();
        for error in [
            fixture.provider.get_canonical_tip_blocking().unwrap_err(),
            fixture
                .runtime
                .block_on(fixture.provider.get_canonical_tip_async())
                .unwrap_err(),
        ] {
            assert!(
                matches!(error, DbError::OLFinalizedClientStateUnavailable { block } if block == state_block)
            );
        }
        for error in fixture.epoch_errors(first.epoch()) {
            assert!(
                matches!(error, DbError::OLFinalizedClientStateUnavailable { block } if block == state_block)
            );
        }
        fixture
            .storage
            .client_state()
            .del_update_blocking(&state_block)
            .unwrap();
        fixture.assert_tip(Some(fixture.genesis));
    }

    #[test]
    fn test_genesis_resolves_with_orphaned_client_state() {
        let fixture = Fixture::new();
        let first = fixture.observe_after(fixture.genesis, 2);
        let state_block = fixture.finalize(first);
        fixture
            .storage
            .l1()
            .revert_canonical_chain(state_block.height() - 1)
            .unwrap();
        fixture.assert_epoch(0, Some(fixture.genesis));
    }

    #[test]
    fn test_orphaned_unfinalized_client_state_does_not_fall_back_to_genesis() {
        let fixture = Fixture::new();
        let block = fixture.advance_l1();
        fixture
            .storage
            .client_state()
            .put_update_blocking(
                &block,
                ClientUpdateOutput::new_state(ClientState::new(None, None)),
            )
            .unwrap();
        fixture.assert_tip(Some(fixture.genesis));
        fixture
            .storage
            .l1()
            .revert_canonical_chain(block.height() - 1)
            .unwrap();
        let replacement = L1BlockId::from(Buf32::from([0x99; 32]));
        fixture
            .storage
            .l1()
            .extend_canonical_chain(&replacement, block.height())
            .unwrap();
        for error in [
            fixture.provider.get_canonical_tip_blocking().unwrap_err(),
            fixture
                .runtime
                .block_on(fixture.provider.get_canonical_tip_async())
                .unwrap_err(),
        ] {
            assert!(
                matches!(error, DbError::OLFinalizedClientStateUnavailable { block: found } if found == block)
            );
        }
    }

    #[test]
    fn test_out_of_order_terminal_rejected() {
        for slot_offset in [0, 1] {
            let fixture = Fixture::new();
            let first = fixture.observe_after(fixture.genesis, 2);
            let second = fixture.observe_after(first, 3);
            fixture.finalize(second);
            fixture
                .storage
                .ol_checkpoint()
                .del_checkpoint_l1_ref_blocking(first)
                .unwrap();
            let invalid = EpochCommitment::new(
                first.epoch(),
                second.last_slot() + slot_offset,
                *first.last_blkid(),
            );
            fixture.store_observation(invalid, fixture.advance_l1());
            for error in fixture.epoch_errors(1) {
                assert!(
                    matches!(error, DbError::OLFinalizedHistoryOrder { earlier, later } if earlier == invalid && later == second),
                    "slot_offset {slot_offset}: {error:?}"
                );
            }
        }
    }

    #[test]
    fn test_ambiguous_genesis_rejected() {
        let fixture = Fixture::new();
        fixture
            .storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(EpochSummary::new(
                0,
                OLBlockCommitment::new(0, Buf32::from([9; 32]).into()),
                OLBlockCommitment::null(),
                L1BlockCommitment::new(100, Buf32::from([100; 32]).into()),
                Buf32::from([9; 32]),
            ))
            .unwrap();
        for error in fixture.epoch_errors(0) {
            assert!(matches!(
                error,
                DbError::OLCheckpointHistoryAmbiguous { epoch: 0 }
            ));
        }
    }

    #[test]
    fn test_malformed_genesis_rejected() {
        for terminal in [
            OLBlockCommitment::null(),
            OLBlockCommitment::new(1, Buf32::from([1; 32]).into()),
        ] {
            let fixture = Fixture::empty();
            fixture
                .storage
                .ol_checkpoint()
                .insert_epoch_summary_blocking(EpochSummary::new(
                    0,
                    terminal,
                    OLBlockCommitment::null(),
                    L1BlockCommitment::new(100, Buf32::from([100; 32]).into()),
                    Buf32::from([1; 32]),
                ))
                .unwrap();
            for error in fixture.epoch_errors(0) {
                assert!(matches!(
                    error,
                    DbError::OLFinalizedHistoryUnavailable { epoch: 0 }
                ));
            }
        }
    }

    struct FailingClientStateDb {
        initialized: AtomicBool,
    }

    impl ClientStateDatabase for FailingClientStateDb {
        fn get_latest_client_state(&self) -> DbResult<Option<(L1BlockCommitment, ClientState)>> {
            if self.initialized.swap(true, Ordering::SeqCst) {
                Err(DbError::Busy)
            } else {
                Ok(None)
            }
        }
        fn put_client_update(&self, _: L1BlockCommitment, _: ClientUpdateOutput) -> DbResult<()> {
            unreachable!("provider must not write client state")
        }
        fn get_client_update(&self, _: L1BlockCommitment) -> DbResult<Option<ClientUpdateOutput>> {
            unreachable!("provider reads the current state")
        }
        fn del_client_update(&self, _: L1BlockCommitment) -> DbResult<()> {
            unreachable!("provider must not delete client state")
        }
        fn get_client_updates_from(
            &self,
            _: L1BlockCommitment,
            _: usize,
        ) -> DbResult<Vec<(L1BlockCommitment, ClientUpdateOutput)>> {
            unreachable!("provider must not scan client state history")
        }
    }

    #[test]
    fn test_client_state_read_failures_propagate() {
        let mut fixture = Fixture::new();
        // Allow manager initialization, then fail the read used to capture each query's bound.
        fixture.provider.client_state = Arc::new(
            ClientStateManager::new(
                fixture.runtime.handle().clone(),
                Arc::new(FailingClientStateDb {
                    initialized: AtomicBool::new(false),
                }),
            )
            .unwrap(),
        );
        fixture.assert_epoch(0, Some(fixture.genesis));
        for error in [
            fixture.provider.get_canonical_tip_blocking().unwrap_err(),
            fixture
                .provider
                .get_canonical_epoch_commitment_at_blocking(1)
                .unwrap_err(),
            fixture
                .provider
                .get_canonical_block_at_blocking(0)
                .unwrap_err(),
            fixture
                .runtime
                .block_on(fixture.provider.get_canonical_tip_async())
                .unwrap_err(),
            fixture
                .runtime
                .block_on(fixture.provider.get_canonical_epoch_commitment_at_async(1))
                .unwrap_err(),
            fixture
                .runtime
                .block_on(fixture.provider.get_canonical_block_at_async(0))
                .unwrap_err(),
        ] {
            assert!(matches!(error, DbError::Busy));
        }
    }
}
