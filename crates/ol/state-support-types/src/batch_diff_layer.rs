//! Read-only state accessor that overlays WriteBatch diffs on a base state.
//!
//! This provides an `IStateAccessor` implementation that checks a stack of
//! `WriteBatch` references before falling back to a base state. All write
//! operations are unsupported since this is read-only.

use std::fmt;

use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount, Mmr64};
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::{IStateAccessor, StateResult};
use strata_ol_state_types::WriteBatch;

use crate::write_tracking_layer::IComputeStateRootWithWrites;

/// A read-only state accessor that overlays a stack of WriteBatch diffs.
///
/// Reads check each batch in reverse order (last = most recent), then fall back
/// to the base state. All write operations return `AcctError::Unsupported` or
/// silently no-op (for setters that return `()`).
///
/// The batch slice can be empty, making this a read-only wrapper for the base.
/// This is useful for scenarios where you want to view state with pending
/// changes applied without modifying anything.
#[derive(Clone)]
pub struct BatchDiffState<'batches, 'base, S: IStateAccessor> {
    base: &'base S,
    write_batches: &'batches [WriteBatch<S::AccountState>],

    /// Helper field so that we only have to compute this once.
    new_accounts: usize,
}

impl<S: IStateAccessor> fmt::Debug for BatchDiffState<'_, '_, S>
where
    S: fmt::Debug,
    S::AccountState: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BatchDiffState")
            .field("base", &self.base)
            .field("write_batches", &self.write_batches)
            .field("new_accounts", &self.new_accounts)
            .finish()
    }
}

impl<'batches, 'base, S: IStateAccessor> BatchDiffState<'batches, 'base, S> {
    /// Creates a new batch diff state wrapping the given base state with a stack of batches.
    ///
    /// The batches are checked in reverse order (last = most recent) before falling
    /// back to the base state. An empty batch slice results in a pure read-only
    /// passthrough to the base.
    pub fn new(base: &'base S, batches: &'batches [WriteBatch<S::AccountState>]) -> Self {
        let new_accounts = batches
            .iter()
            .map(|wb| wb.ledger().new_accounts().len())
            .sum();

        Self {
            base,
            write_batches: batches,
            new_accounts,
        }
    }

    /// Returns a reference to the base state.
    pub fn base(&self) -> &'base S {
        self.base
    }

    /// Returns a reference to the batch slice.
    pub fn write_batches(&self) -> &'batches [WriteBatch<S::AccountState>] {
        self.write_batches
    }

    /// Returns the total number of new accounts added by writes in the layer.
    pub fn new_accounts(&self) -> usize {
        self.new_accounts
    }

    /// Internal function for helping with lookups.
    ///
    /// Walks the batch stack newest-first, returning the first `Some` produced
    /// by `on_wb`, or if none match, returns the result of `on_base`.
    fn resolve<T>(
        &self,
        on_wb: impl Fn(&'batches WriteBatch<S::AccountState>) -> Option<T>,
        on_base: impl FnOnce() -> T,
    ) -> T {
        for wb in self.write_batches.iter().rev() {
            if let Some(v) = on_wb(wb) {
                return v;
            }
        }
        on_base()
    }
}

impl<'batches, 'base, S: IStateAccessor + IComputeStateRootWithWrites> IStateAccessor
    for BatchDiffState<'batches, 'base, S>
{
    type AccountState = S::AccountState;

    // ===== Global state methods =====

    fn cur_slot(&self) -> u64 {
        self.resolve(|b| b.global_writes().cur_slot, || self.base.cur_slot())
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.resolve(|b| b.epochal_writes().cur_epoch, || self.base.cur_epoch())
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.resolve(
            |b| b.epochal_writes().last_l1_blkid.as_ref(),
            || self.base.last_l1_blkid(),
        )
    }

    fn last_l1_height(&self) -> L1Height {
        self.resolve(
            |b| b.epochal_writes().last_l1_height,
            || self.base.last_l1_height(),
        )
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.resolve(
            |b| b.epochal_writes().asm_recorded_epoch.as_ref(),
            || self.base.asm_recorded_epoch(),
        )
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.resolve(
            |b| b.epochal_writes().total_ledger_balance,
            || self.base.total_ledger_balance(),
        )
    }

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        self.resolve(
            |b| b.epochal_writes().asm_manifests_mmr.as_ref(),
            || self.base.asm_manifests_mmr(),
        )
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> StateResult<bool> {
        self.resolve(
            |b| b.ledger().contains_account(&id).then_some(Ok(true)),
            || self.base.check_account_exists(id),
        )
    }

    fn get_account_state(&self, id: AccountId) -> StateResult<Option<&Self::AccountState>> {
        self.resolve(
            |b| b.ledger().get_account(&id).map(|s| Ok(Some(s))),
            || self.base.get_account_state(id),
        )
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> StateResult<Option<AccountId>> {
        self.resolve(
            |b| b.ledger().find_id_by_serial(serial).map(|id| Ok(Some(id))),
            || self.base.find_account_id_by_serial(serial),
        )
    }

    fn next_account_serial(&self) -> AccountSerial {
        let base_serial: u32 = self.base.next_account_serial().into();
        AccountSerial::from(base_serial + self.new_accounts as u32)
    }

    fn compute_state_root(&self) -> StateResult<Buf32> {
        self.base
            .compute_state_root_with_writes(self.write_batches.iter())
    }
}

impl<'batches, 'base, S: IComputeStateRootWithWrites> IComputeStateRootWithWrites
    for BatchDiffState<'batches, 'base, S>
{
    fn compute_state_root_with_writes<'b>(
        &'b self,
        writes: impl Iterator<Item = &'b WriteBatch<Self::AccountState>>,
    ) -> StateResult<Buf32>
    where
        Self::AccountState: 'b,
    {
        self.base
            .compute_state_root_with_writes(self.write_batches.iter().chain(writes))
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{BitcoinAmount, SYSTEM_RESERVED_ACCTS};
    use strata_identifiers::{AccountSerial, Buf32, Epoch, L1BlockCommitment, L1BlockId, Slot};
    use strata_ledger_types::{IAccountState, IStateAccessor};
    use strata_ol_params::OLParams;
    use strata_ol_state_types::OLState;

    use super::*;
    use crate::{memory_state_layer::MemoryStateBaseLayer, test_utils::*};

    fn new_layer_at(epoch: Epoch, slot: Slot) -> MemoryStateBaseLayer {
        let mut params = OLParams::new_empty(L1BlockCommitment::default());
        params.header.slot = slot;
        params.header.epoch = epoch;
        let state = OLState::from_genesis_params(&params)
            .expect("failed to create OLState from genesis params");
        MemoryStateBaseLayer::new(state)
    }

    // =========================================================================
    // Empty batch tests (pure passthrough)
    // =========================================================================

    #[test]
    fn test_read_from_base_when_empty_batches() {
        let account_id = test_account_id(1);
        let (base_layer, serial) =
            setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

        let batches: Vec<WriteBatch<_>> = vec![];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        // Should read from base
        let account = diff_state.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));
    }

    #[test]
    fn test_global_state_from_base_when_empty() {
        let base_layer = new_layer_at(5, 100);
        let batches: Vec<WriteBatch<_>> = vec![];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        assert_eq!(diff_state.cur_slot(), 100);
        assert_eq!(diff_state.cur_epoch(), 5);
    }

    #[test]
    fn test_check_account_exists_in_base_only() {
        let account_id = test_account_id(1);
        let nonexistent_id = test_account_id(99);
        let (base_layer, _) =
            setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

        let batches: Vec<WriteBatch<_>> = vec![];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        assert!(diff_state.check_account_exists(account_id).unwrap());
        assert!(!diff_state.check_account_exists(nonexistent_id).unwrap());
    }

    // =========================================================================
    // Single batch tests
    // =========================================================================

    #[test]
    fn test_read_from_single_batch() {
        let account_id = test_account_id(1);
        let base_layer = create_test_base_layer();

        // Create a batch with an account
        let mut batch = WriteBatch::default();
        let snark_state = test_snark_account_state(1);
        let new_acct = test_new_snark_account_data(&snark_state, BitcoinAmount::from_sat(5000));
        let serial = base_layer.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        // Should read from batch
        let account = diff_state.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));
    }

    #[test]
    fn test_check_account_exists_in_batch() {
        let account_id = test_account_id(1);
        let base_layer = create_test_base_layer();

        let mut batch = WriteBatch::default();
        let snark_state = test_snark_account_state(1);
        let new_acct = test_new_snark_account_data(&snark_state, BitcoinAmount::from_sat(5000));
        let serial = base_layer.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        assert!(diff_state.check_account_exists(account_id).unwrap());
    }

    #[test]
    fn test_global_state_from_top_batch() {
        let base_layer = new_layer_at(5, 100);

        let mut batch = WriteBatch::default();
        batch.global_writes_mut().cur_slot = Some(200);
        batch.epochal_writes_mut().cur_epoch = Some(10);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        assert_eq!(diff_state.cur_slot(), 200);
        assert_eq!(diff_state.cur_epoch(), 10);
    }

    // =========================================================================
    // Batch stack tests (multiple batches)
    // =========================================================================

    #[test]
    fn test_read_from_batch_stack_last_shadows() {
        let account_id = test_account_id(1);
        let base_layer = create_test_base_layer();

        // First batch: account with 1000 sats
        let mut batch1 = WriteBatch::default();
        let snark_state1 = test_snark_account_state(1);
        let new_acct1 = test_new_snark_account_data(&snark_state1, BitcoinAmount::from_sat(1000));
        let serial1 = base_layer.next_account_serial();
        batch1
            .ledger_mut()
            .create_account_from_data(account_id, new_acct1, serial1);

        // Second batch (more recent): same account with 5000 sats
        // This batch shadows the first, so uses a different serial
        let mut batch2 = WriteBatch::default();
        let snark_state2 = test_snark_account_state(2);
        let new_acct2 = test_new_snark_account_data(&snark_state2, BitcoinAmount::from_sat(5000));
        let serial2 = AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1);
        batch2
            .ledger_mut()
            .create_account_from_data(account_id, new_acct2, serial2);

        // Last batch should shadow first
        let batches = vec![batch1, batch2];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        let account = diff_state.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));
    }

    #[test]
    fn test_read_falls_through_to_earlier_batch() {
        let account_id_1 = test_account_id(1);
        let account_id_2 = test_account_id(2);
        let base_layer = create_test_base_layer();

        // First batch: account 1
        let mut batch1 = WriteBatch::default();
        let snark_state1 = test_snark_account_state(1);
        let new_acct1 = test_new_snark_account_data(&snark_state1, BitcoinAmount::from_sat(1000));
        let serial1 = base_layer.next_account_serial();
        batch1
            .ledger_mut()
            .create_account_from_data(account_id_1, new_acct1, serial1);

        // Second batch: account 2 only
        let mut batch2 = WriteBatch::default();
        let snark_state2 = test_snark_account_state(2);
        let new_acct2 = test_new_snark_account_data(&snark_state2, BitcoinAmount::from_sat(2000));
        let serial2 = AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1);
        batch2
            .ledger_mut()
            .create_account_from_data(account_id_2, new_acct2, serial2);

        let batches = vec![batch1, batch2];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        // Account 1 should be found in batch1 (falls through from batch2)
        let account1 = diff_state.get_account_state(account_id_1).unwrap().unwrap();
        assert_eq!(account1.balance(), BitcoinAmount::from_sat(1000));

        // Account 2 should be found in batch2
        let account2 = diff_state.get_account_state(account_id_2).unwrap().unwrap();
        assert_eq!(account2.balance(), BitcoinAmount::from_sat(2000));
    }

    #[test]
    fn test_read_falls_through_to_base() {
        let account_id_base = test_account_id(1);
        let account_id_batch = test_account_id(2);
        let (base_layer, _) =
            setup_layer_with_snark_account(account_id_base, 1, BitcoinAmount::from_sat(1000));

        let mut batch = WriteBatch::default();
        let snark_state = test_snark_account_state(2);
        let new_acct = test_new_snark_account_data(&snark_state, BitcoinAmount::from_sat(2000));
        let serial = base_layer.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id_batch, new_acct, serial);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        // Account in base should be found
        let base_account = diff_state
            .get_account_state(account_id_base)
            .unwrap()
            .unwrap();
        assert_eq!(base_account.balance(), BitcoinAmount::from_sat(1000));

        // Account in batch should also be found
        let batch_account = diff_state
            .get_account_state(account_id_batch)
            .unwrap()
            .unwrap();
        assert_eq!(batch_account.balance(), BitcoinAmount::from_sat(2000));
    }

    #[test]
    fn test_find_serial_in_batch_stack() {
        let account_id = test_account_id(1);
        let base_layer = create_test_base_layer();

        let mut batch = WriteBatch::default();
        let snark_state = test_snark_account_state(1);
        let new_acct = test_new_snark_account_data(&snark_state, BitcoinAmount::from_sat(1000));
        let serial = base_layer.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        let found_id = diff_state.find_account_id_by_serial(serial).unwrap();
        assert_eq!(found_id, Some(account_id));
    }

    // =========================================================================
    // Epochal state tests
    // =========================================================================

    #[test]
    fn test_epochal_state_from_top_batch() {
        let base_layer = create_test_base_layer();

        let mut batch = WriteBatch::default();
        batch.epochal_writes_mut().total_ledger_balance = Some(BitcoinAmount::from_sat(1_000_000));

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        assert_eq!(
            diff_state.total_ledger_balance(),
            BitcoinAmount::from_sat(1_000_000)
        );
    }

    #[test]
    fn test_global_epochal_fall_through_batch_stack() {
        // Older batch sets cur_slot/cur_epoch/last_l1_blkid; newer batch only
        // touches total_ledger_balance. The newer batch must not shadow the
        // unrelated fields set by the older batch.
        let base_layer = new_layer_at(1, 10);

        let older_blkid = L1BlockId::from(Buf32::from([0x11u8; 32]));

        let mut batch1 = WriteBatch::default();
        batch1.global_writes_mut().cur_slot = Some(200);
        batch1.epochal_writes_mut().cur_epoch = Some(7);
        batch1.epochal_writes_mut().last_l1_blkid = Some(older_blkid);

        let mut batch2 = WriteBatch::default();
        batch2.epochal_writes_mut().total_ledger_balance = Some(BitcoinAmount::from_sat(42));

        let batches = vec![batch1, batch2];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        assert_eq!(diff_state.cur_slot(), 200);
        assert_eq!(diff_state.cur_epoch(), 7);
        assert_eq!(*diff_state.last_l1_blkid(), older_blkid);
        assert_eq!(
            diff_state.total_ledger_balance(),
            BitcoinAmount::from_sat(42)
        );
    }

    #[test]
    fn test_state_root_changes_with_writes() {
        let account_id = test_account_id(1);
        let base_layer = create_test_base_layer();

        let base_root = base_layer.compute_state_root().unwrap();

        let mut batch = WriteBatch::default();
        let snark_state = test_snark_account_state(1);
        let new_acct = test_new_snark_account_data(&snark_state, BitcoinAmount::from_sat(5000));
        let serial = base_layer.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_layer, &batches);
        let diff_root = diff_state.compute_state_root().unwrap();

        assert_ne!(base_root, diff_root);

        // An empty-batches diff should match the base root.
        let empty_batches: Vec<WriteBatch<_>> = vec![];
        let passthrough = BatchDiffState::new(&base_layer, &empty_batches);
        assert_eq!(passthrough.compute_state_root().unwrap(), base_root);
    }

    #[test]
    fn test_stacked_batch_diff_layers() {
        let account_id_1 = test_account_id(1);
        let account_id_2 = test_account_id(2);
        let base_layer = create_test_base_layer();

        // Inner layer adds account 1.
        let mut batch1 = WriteBatch::default();
        let snark_state1 = test_snark_account_state(1);
        let new_acct1 = test_new_snark_account_data(&snark_state1, BitcoinAmount::from_sat(1000));
        let serial1 = base_layer.next_account_serial();
        batch1
            .ledger_mut()
            .create_account_from_data(account_id_1, new_acct1, serial1);

        let inner_batches = vec![batch1];
        let inner = BatchDiffState::new(&base_layer, &inner_batches);

        // Outer layer stacks another batch adding account 2 on top of `inner`.
        let mut batch2 = WriteBatch::default();
        let snark_state2 = test_snark_account_state(2);
        let new_acct2 = test_new_snark_account_data(&snark_state2, BitcoinAmount::from_sat(2000));
        let serial2 = inner.next_account_serial();
        batch2
            .ledger_mut()
            .create_account_from_data(account_id_2, new_acct2, serial2);

        let outer_batches = vec![batch2];
        let outer = BatchDiffState::new(&inner, &outer_batches);

        // Both accounts are visible through the outer layer.
        let acct1 = outer.get_account_state(account_id_1).unwrap().unwrap();
        assert_eq!(acct1.balance(), BitcoinAmount::from_sat(1000));
        let acct2 = outer.get_account_state(account_id_2).unwrap().unwrap();
        assert_eq!(acct2.balance(), BitcoinAmount::from_sat(2000));

        // Serial lookups resolve through both layers.
        assert_eq!(
            outer.find_account_id_by_serial(serial1).unwrap(),
            Some(account_id_1)
        );
        assert_eq!(
            outer.find_account_id_by_serial(serial2).unwrap(),
            Some(account_id_2)
        );

        // next_account_serial accumulates new accounts from both layers.
        let base_next: u32 = base_layer.next_account_serial().into();
        let outer_next: u32 = outer.next_account_serial().into();
        assert_eq!(outer_next, base_next + 2);

        // Stacking more writes changes the state root relative to the inner layer.
        assert_ne!(
            inner.compute_state_root().unwrap(),
            outer.compute_state_root().unwrap()
        );
    }

    #[test]
    fn test_last_l1_blkid_from_batch() {
        let base_layer = create_test_base_layer();
        let batch: WriteBatch<_> = WriteBatch::default();

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_layer, &batches);

        // Should return the L1 block ID from the batch's epochal state
        let blkid = diff_state.last_l1_blkid();
        assert_eq!(*blkid, L1BlockId::from(Buf32::zero()));
    }
}
