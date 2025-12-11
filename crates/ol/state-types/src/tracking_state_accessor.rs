//! Generic tracking accessor that wraps any StateAccessor and tracks all modifications
//! during block execution, accumulating them into a WriteBatch.

use std::fmt;

use strata_acct_types::{
    AcctError, AccountId, AccountSerial, AccountTypeId, AcctResult, BitcoinAmount, Hash,
};
use strata_identifiers::{Buf32, EpochCommitment, L1Height};
use strata_ledger_types::{
    AccountTypeState, AsmManifest, Coin, IAccountState, IGlobalState, IL1ViewState,
    ISnarkAccountState, StateAccessor,
};
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::writebatch::{ExecutionAuxiliaryData, WriteBatch};

/// Tracks all state changes for WriteBatch generation using CoW overlay.
/// Generic over any StateAccessor implementation.
pub struct TrackingStateAccessor<S: StateAccessor> {
    /// Base state before execution
    base: S,

    /// Copy-on-Write overlay tracking modifications during execution
    writebatch: WriteBatch<S>,

    /// Accumulate auxiliary data for database persistence
    aux: ExecutionAuxiliaryData,
}

impl<S: StateAccessor + fmt::Debug> fmt::Debug for TrackingStateAccessor<S>
where
    S::GlobalState: fmt::Debug,
    S::L1ViewState: fmt::Debug,
    S::AccountState: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackingStateAccessor")
            .field("base", &self.base)
            .field("writebatch", &self.writebatch)
            .field("aux", &self.aux)
            .finish()
    }
}

impl<S: StateAccessor> TrackingStateAccessor<S> {
    /// Create a new state accessor from an initial state
    pub fn new(state: S) -> Self {
        let next_serial = state.get_next_serial();
        Self {
            base: state,
            writebatch: WriteBatch::new(next_serial),
            aux: ExecutionAuxiliaryData::default(),
        }
    }

    /// Finalize execution and produce WriteBatch and auxiliary data
    pub fn finalize_as_writebatch(self) -> (WriteBatch<S>, ExecutionAuxiliaryData) {
        (self.writebatch, self.aux)
    }

    /// Get reference to the base state (before modifications)
    pub fn base_state(&self) -> &S {
        &self.base
    }

    /// Get reference to the writebatch overlay
    pub fn writebatch(&self) -> &WriteBatch<S> {
        &self.writebatch
    }

    /// Get mutable account, cloning from base if not in overlay
    fn get_account_mut(&mut self, acct_id: AccountId) -> AcctResult<S::AccountState>
    where
        S::AccountState: Clone,
    {
        if let Some(acct) = self.writebatch.get_account(&acct_id) {
            Ok(acct.clone())
        } else {
            self.base
                .get_account_state(acct_id)?
                .ok_or(AcctError::MissingExpectedAccount(acct_id))
                .cloned()
        }
    }

    /// Extract snark account state from account
    fn get_snark_state_mut(
        acct: &mut S::AccountState,
    ) -> AcctResult<<S::AccountState as IAccountState>::SnarkAccountState>
    where
        S::AccountState: IAccountState,
    {
        match acct.get_type_state()? {
            AccountTypeState::Snark(s) => Ok(s),
            _ => Err(AcctError::MismatchedType(acct.ty()?, AccountTypeId::Snark)),
        }
    }
}

impl<S: StateAccessor> StateAccessor for TrackingStateAccessor<S>
where
    S::GlobalState: Clone,
    S::L1ViewState: Clone,
    S::AccountState: Clone + IAccountState,
{
    type GlobalState = S::GlobalState;
    type L1ViewState = S::L1ViewState;
    type AccountState = S::AccountState;

    fn global(&self) -> &Self::GlobalState {
        self.writebatch
            .global_state()
            .unwrap_or_else(|| self.base.global())
    }

    fn set_cur_slot(&mut self, slot: u64) {
        let global = self.writebatch.global_state_mut_or_insert(self.base.global());
        global.set_cur_slot(slot);
    }

    fn l1_view(&self) -> &Self::L1ViewState {
        self.writebatch
            .epochal_state()
            .unwrap_or_else(|| self.base.l1_view())
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        let l1_view = self.writebatch.epochal_state_mut_or_insert(self.base.l1_view());
        l1_view.set_cur_epoch(epoch);
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        // Accumulate for auxiliary data
        self.aux.asm_manifests.push(mf.clone());

        // Apply to overlay
        let l1_view = self.writebatch.epochal_state_mut_or_insert(self.base.l1_view());
        l1_view.append_manifest(height, mf);
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        let l1_view = self.writebatch.epochal_state_mut_or_insert(self.base.l1_view());
        l1_view.set_asm_recorded_epoch(epoch);
    }

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        if self.writebatch.has_account(&id) {
            Ok(true)
        } else {
            self.base.check_account_exists(id)
        }
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        if let Some(acct) = self.writebatch.get_account(&id) {
            Ok(Some(acct))
        } else {
            self.base.get_account_state(id)
        }
    }

    fn add_balance(&mut self, acct_id: AccountId, coin: Coin) -> AcctResult<()> {
        let mut acct = match self.get_account_mut(acct_id) {
            Ok(acct) => acct,
            Err(e) => {
                // Consume coin to avoid Drop panic
                coin.safely_consume_unchecked();
                return Err(e);
            }
        };
        acct.add_balance(coin);
        self.writebatch.insert_account(acct_id, acct);
        Ok(())
    }

    fn take_balance(&mut self, acct_id: AccountId, amt: BitcoinAmount) -> AcctResult<Coin> {
        let mut acct = self.get_account_mut(acct_id)?;
        let coin = acct.take_balance(amt)?;
        self.writebatch.insert_account(acct_id, acct);
        Ok(coin)
    }

    fn insert_inbox_message(&mut self, acct_id: AccountId, entry: MessageEntry) -> AcctResult<()> {
        self.aux
            .account_message_additions
            .entry(acct_id)
            .or_default()
            .push(entry.clone());

        let mut acct = self.get_account_mut(acct_id)?;
        let mut snark_state = Self::get_snark_state_mut(&mut acct)?;
        snark_state.insert_inbox_message(entry)?;
        acct.set_type_state(AccountTypeState::Snark(snark_state))?;
        self.writebatch.insert_account(acct_id, acct);
        Ok(())
    }

    fn set_proof_state_directly(
        &mut self,
        acct_id: AccountId,
        state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
    ) -> AcctResult<()> {
        let mut acct = self.get_account_mut(acct_id)?;
        let mut snark_state = Self::get_snark_state_mut(&mut acct)?;
        snark_state.set_proof_state_directly(state, next_read_idx, seqno);
        acct.set_type_state(AccountTypeState::Snark(snark_state))?;
        self.writebatch.insert_account(acct_id, acct);
        Ok(())
    }

    fn update_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()> {
        self.writebatch.insert_account(id, state);
        Ok(())
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        // Check if account already exists in overlay or base
        if self.writebatch.has_account(&id) || self.base.check_account_exists(id)? {
            return Err(AcctError::AccountIdExists(id));
        }

        // Get the next serial from writebatch (no base mutation!)
        let serial = self.writebatch.get_next_serial();

        // Create the account state using the trait constructor with zero initial balance
        let account_state = S::AccountState::new_account(serial, BitcoinAmount::from(0), state);

        // Add to writebatch - purely in overlay, no base mutation!
        let assigned_serial = self.writebatch.create_account(id, account_state);

        Ok(assigned_serial)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        // Check overlay first for newly created accounts
        if let Some(id) = self.writebatch.find_serial(serial) {
            return Ok(Some(id));
        }
        // Fall back to base state
        self.base.find_account_id_by_serial(serial)
    }

    fn get_next_serial(&self) -> AccountSerial {
        self.writebatch.get_next_serial()
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        // TODO: This needs to compute state root incorporating overlay changes
        // For now, delegate to base (incorrect but will compile)
        self.base.compute_state_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OLState, account::NativeAccountTypeState};
    use strata_acct_types::BitcoinAmount;
    use strata_identifiers::Buf32;
    use strata_ledger_types::{AccountTypeState, Coin, IAccountState};

    // Test helpers
    fn create_test_state() -> OLState {
        OLState::new_genesis()
    }

    fn test_account_id(n: u8) -> AccountId {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        AccountId::from(bytes)
    }

    fn create_empty_account_type() -> AccountTypeState<crate::account::AccountState> {
        AccountTypeState::Empty
    }

    fn assert_base_unchanged(before: &OLState, after: &OLState) {
        assert_eq!(before.get_next_serial(), after.get_next_serial());
        // Can add more comprehensive checks if needed
    }

    // ===== P1: Base Immutability Tests =====

    #[test]
    fn test_base_state_unchanged_after_create() {
        let base = create_test_state();
        let base_clone = base.clone();

        let mut tracker = TrackingStateAccessor::new(base);
        let id = test_account_id(1);

        tracker.create_new_account(id, create_empty_account_type()).unwrap();

        // Base should be unchanged
        assert_base_unchanged(&base_clone, tracker.base_state());
        assert!(!tracker.base_state().check_account_exists(id).unwrap());
    }

    #[test]
    fn test_base_serial_unchanged_after_operations() {
        let base = create_test_state();
        let initial_serial = base.get_next_serial();

        let mut tracker = TrackingStateAccessor::new(base);

        // Create multiple accounts
        for i in 1..=5 {
            tracker.create_new_account(test_account_id(i), create_empty_account_type()).unwrap();
        }

        // Base serial should be unchanged
        assert_eq!(tracker.base_state().get_next_serial(), initial_serial);
    }

    // ===== P2: Serial Determinism Tests =====

    #[test]
    fn test_serial_allocation_sequential() {
        let base = create_test_state();
        let starting_serial = base.get_next_serial();

        let mut tracker = TrackingStateAccessor::new(base);

        let serial1 = tracker.create_new_account(test_account_id(1), create_empty_account_type()).unwrap();
        let serial2 = tracker.create_new_account(test_account_id(2), create_empty_account_type()).unwrap();
        let serial3 = tracker.create_new_account(test_account_id(3), create_empty_account_type()).unwrap();

        assert_eq!(serial1, starting_serial);
        assert_eq!(serial2, AccountSerial::from(u32::from(starting_serial) + 1));
        assert_eq!(serial3, AccountSerial::from(u32::from(starting_serial) + 2));
    }

    #[test]
    fn test_multiple_trackers_same_starting_serial() {
        let base = create_test_state();
        let starting_serial = base.get_next_serial();

        let mut tracker1 = TrackingStateAccessor::new(base.clone());
        let mut tracker2 = TrackingStateAccessor::new(base);

        let serial1 = tracker1.create_new_account(test_account_id(1), create_empty_account_type()).unwrap();
        let serial2 = tracker2.create_new_account(test_account_id(2), create_empty_account_type()).unwrap();

        // Both should start from the same serial
        assert_eq!(serial1, starting_serial);
        assert_eq!(serial2, starting_serial);
    }

    #[test]
    fn test_applied_serials_match_allocated() {
        let mut base = create_test_state();
        let starting_serial = base.get_next_serial();

        let mut tracker = TrackingStateAccessor::new(base.clone());

        let id1 = test_account_id(1);
        let id2 = test_account_id(2);

        let serial1 = tracker.create_new_account(id1, create_empty_account_type()).unwrap();
        let serial2 = tracker.create_new_account(id2, create_empty_account_type()).unwrap();

        let (writebatch, _) = tracker.finalize_as_writebatch();
        base.apply_write_batch(writebatch).unwrap();

        // Verify serials in base match what was allocated
        assert_eq!(base.find_account_id_by_serial(serial1).unwrap(), Some(id1));
        assert_eq!(base.find_account_id_by_serial(serial2).unwrap(), Some(id2));
        assert_eq!(base.get_account_state(id1).unwrap().unwrap().serial(), serial1);
        assert_eq!(base.get_account_state(id2).unwrap().unwrap().serial(), serial2);
    }

    // ===== P3: State Freshness Tests (Critical Bug Regression) =====

    #[test]
    fn test_create_then_modify_balance() {
        let mut base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base.clone());

        let id = test_account_id(1);
        tracker.create_new_account(id, create_empty_account_type()).unwrap();

        // Modify the balance
        let coin = Coin::new_unchecked(BitcoinAmount::from(1000));
        tracker.add_balance(id, coin).unwrap();

        let (writebatch, _) = tracker.finalize_as_writebatch();
        base.apply_write_batch(writebatch).unwrap();

        // Verify balance was preserved
        let acct = base.get_account_state(id).unwrap().unwrap();
        assert_eq!(acct.balance(), BitcoinAmount::from(1000));
    }

    #[test]
    fn test_create_modify_modify() {
        let mut base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base.clone());

        let id = test_account_id(1);
        tracker.create_new_account(id, create_empty_account_type()).unwrap();

        // Multiple modifications
        tracker.add_balance(id, Coin::new_unchecked(BitcoinAmount::from(100))).unwrap();
        tracker.add_balance(id, Coin::new_unchecked(BitcoinAmount::from(200))).unwrap();
        tracker.add_balance(id, Coin::new_unchecked(BitcoinAmount::from(300))).unwrap();

        let (writebatch, _) = tracker.finalize_as_writebatch();
        base.apply_write_batch(writebatch).unwrap();

        // Verify final state has all modifications
        let acct = base.get_account_state(id).unwrap().unwrap();
        assert_eq!(acct.balance(), BitcoinAmount::from(600));
    }

    #[test]
    fn test_created_account_later_modifications_preserved() {
        // This is the exact bug we fixed: created_accounts storing stale state
        let mut base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base.clone());

        let id = test_account_id(1);
        let serial = tracker.create_new_account(id, create_empty_account_type()).unwrap();

        // Immediately modify after creation
        tracker.add_balance(id, Coin::new_unchecked(BitcoinAmount::from(500))).unwrap();

        let (writebatch, _) = tracker.finalize_as_writebatch();

        // Verify writebatch has the modified state, not creation state
        let acct = writebatch.modified_accounts.get(&id).unwrap();
        assert_eq!(acct.balance(), BitcoinAmount::from(500));
        assert_eq!(acct.serial(), serial);

        base.apply_write_batch(writebatch).unwrap();

        // Verify applied state has the modification
        let acct = base.get_account_state(id).unwrap().unwrap();
        assert_eq!(acct.balance(), BitcoinAmount::from(500));
    }

    // ===== P4: Overlay Visibility Tests =====

    #[test]
    fn test_check_account_exists_sees_overlay() {
        let base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base);

        let id = test_account_id(1);
        tracker.create_new_account(id, create_empty_account_type()).unwrap();

        // Should be visible immediately in tracker
        assert!(tracker.check_account_exists(id).unwrap());
        // But not in base
        assert!(!tracker.base_state().check_account_exists(id).unwrap());
    }

    #[test]
    fn test_get_account_state_returns_overlay() {
        let base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base);

        let id = test_account_id(1);
        tracker.create_new_account(id, create_empty_account_type()).unwrap();
        tracker.add_balance(id, Coin::new_unchecked(BitcoinAmount::from(123))).unwrap();

        // Should get overlay state with the balance
        let acct = tracker.get_account_state(id).unwrap().unwrap();
        assert_eq!(acct.balance(), BitcoinAmount::from(123));
    }

    #[test]
    fn test_find_serial_finds_overlay_accounts() {
        let base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base);

        let id = test_account_id(1);
        let serial = tracker.create_new_account(id, create_empty_account_type()).unwrap();

        // Should find the account by serial in overlay
        assert_eq!(tracker.find_account_id_by_serial(serial).unwrap(), Some(id));
        // But not in base
        assert_eq!(tracker.base_state().find_account_id_by_serial(serial).unwrap(), None);
    }

    // ===== P5: Application Correctness Tests =====

    #[test]
    fn test_apply_writebatch_creates_accounts_in_order() {
        let mut base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base.clone());

        let id1 = test_account_id(1);
        let id2 = test_account_id(2);
        let id3 = test_account_id(3);

        let serial1 = tracker.create_new_account(id1, create_empty_account_type()).unwrap();
        let serial2 = tracker.create_new_account(id2, create_empty_account_type()).unwrap();
        let serial3 = tracker.create_new_account(id3, create_empty_account_type()).unwrap();

        let (writebatch, _) = tracker.finalize_as_writebatch();
        base.apply_write_batch(writebatch).unwrap();

        // Verify all accounts exist and have correct serials
        assert_eq!(base.get_account_state(id1).unwrap().unwrap().serial(), serial1);
        assert_eq!(base.get_account_state(id2).unwrap().unwrap().serial(), serial2);
        assert_eq!(base.get_account_state(id3).unwrap().unwrap().serial(), serial3);
    }

    #[test]
    fn test_apply_writebatch_updates_existing_accounts() {
        let mut base = create_test_state();

        // Create an account in base first
        let id = test_account_id(1);
        base.create_new_account(id, create_empty_account_type()).unwrap();

        // Now modify it through tracker
        let mut tracker = TrackingStateAccessor::new(base.clone());
        tracker.add_balance(id, Coin::new_unchecked(BitcoinAmount::from(999))).unwrap();

        let (writebatch, _) = tracker.finalize_as_writebatch();
        base.apply_write_batch(writebatch).unwrap();

        // Verify the update was applied
        let acct = base.get_account_state(id).unwrap().unwrap();
        assert_eq!(acct.balance(), BitcoinAmount::from(999));
    }

    #[test]
    fn test_apply_writebatch_empty() {
        let mut base = create_test_state();
        let base_serial = base.get_next_serial();

        let tracker = TrackingStateAccessor::new(base.clone());
        let (writebatch, _) = tracker.finalize_as_writebatch();

        base.apply_write_batch(writebatch).unwrap();

        // Nothing should change
        assert_eq!(base.get_next_serial(), base_serial);
    }

    #[test]
    fn test_apply_writebatch_with_global_epochal_changes() {
        let mut base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base.clone());

        // Modify global and epochal state
        tracker.set_cur_slot(42);
        tracker.set_cur_epoch(7);

        let (writebatch, _) = tracker.finalize_as_writebatch();
        base.apply_write_batch(writebatch).unwrap();

        // Verify changes were applied
        assert_eq!(base.global().get_cur_slot(), 42);
        assert_eq!(base.l1_view().cur_epoch(), 7);
    }

    // ===== P6: Error Handling Tests =====

    #[test]
    fn test_duplicate_account_creation_fails() {
        let base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base);

        let id = test_account_id(1);
        tracker.create_new_account(id, create_empty_account_type()).unwrap();

        // Second create should fail
        let result = tracker.create_new_account(id, create_empty_account_type());
        assert!(result.is_err());
    }

    #[test]
    fn test_create_existing_account_fails() {
        let mut base = create_test_state();
        let id = test_account_id(1);

        // Create in base
        base.create_new_account(id, create_empty_account_type()).unwrap();

        // Try to create same account in tracker
        let mut tracker = TrackingStateAccessor::new(base);
        let result = tracker.create_new_account(id, create_empty_account_type());
        assert!(result.is_err());
    }

    #[test]
    fn test_modify_nonexistent_account_fails() {
        let base = create_test_state();
        let mut tracker = TrackingStateAccessor::new(base);

        let id = test_account_id(99);
        let coin = Coin::new_unchecked(BitcoinAmount::from(100));
        let result = tracker.add_balance(id, coin);
        assert!(result.is_err());
        // Note: Coin is consumed even on error in the current implementation
    }

    // ===== P7: Isolation Tests =====

    #[test]
    fn test_two_trackers_independent() {
        let base = create_test_state();

        let mut tracker1 = TrackingStateAccessor::new(base.clone());
        let mut tracker2 = TrackingStateAccessor::new(base);

        let id1 = test_account_id(1);
        let id2 = test_account_id(2);

        tracker1.create_new_account(id1, create_empty_account_type()).unwrap();
        tracker2.create_new_account(id2, create_empty_account_type()).unwrap();

        // Each tracker should only see its own account
        assert!(tracker1.check_account_exists(id1).unwrap());
        assert!(!tracker1.check_account_exists(id2).unwrap());

        assert!(tracker2.check_account_exists(id2).unwrap());
        assert!(!tracker2.check_account_exists(id1).unwrap());
    }

    #[test]
    fn test_sequential_apply_correct_serials() {
        let mut base = create_test_state();
        let starting_serial = base.get_next_serial();

        // First tracker creates 2 accounts
        let mut tracker1 = TrackingStateAccessor::new(base.clone());
        let id1 = test_account_id(1);
        let id2 = test_account_id(2);
        tracker1.create_new_account(id1, create_empty_account_type()).unwrap();
        tracker1.create_new_account(id2, create_empty_account_type()).unwrap();

        let (wb1, _) = tracker1.finalize_as_writebatch();
        base.apply_write_batch(wb1).unwrap();

        // Second tracker creates 2 more accounts
        let mut tracker2 = TrackingStateAccessor::new(base.clone());
        let id3 = test_account_id(3);
        let id4 = test_account_id(4);
        tracker2.create_new_account(id3, create_empty_account_type()).unwrap();
        tracker2.create_new_account(id4, create_empty_account_type()).unwrap();

        let (wb2, _) = tracker2.finalize_as_writebatch();
        base.apply_write_batch(wb2).unwrap();

        // Verify all serials are sequential
        let serial1 = base.get_account_state(id1).unwrap().unwrap().serial();
        let serial2 = base.get_account_state(id2).unwrap().unwrap().serial();
        let serial3 = base.get_account_state(id3).unwrap().unwrap().serial();
        let serial4 = base.get_account_state(id4).unwrap().unwrap().serial();

        assert_eq!(serial1, starting_serial);
        assert_eq!(serial2, AccountSerial::from(u32::from(starting_serial) + 1));
        assert_eq!(serial3, AccountSerial::from(u32::from(starting_serial) + 2));
        assert_eq!(serial4, AccountSerial::from(u32::from(starting_serial) + 3));
    }

    // ===== Property Tests =====

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn proptest_serial_allocation_sequential(num_creates in 1usize..20) {
            let base = create_test_state();
            let starting_serial = base.get_next_serial();
            let mut tracker = TrackingStateAccessor::new(base);

            let mut serials = Vec::new();
            for i in 0..num_creates {
                let id = test_account_id(i as u8);
                let serial = tracker.create_new_account(id, create_empty_account_type()).unwrap();
                serials.push(serial);
            }

            // Verify all serials are sequential
            for (i, serial) in serials.iter().enumerate() {
                prop_assert_eq!(
                    *serial,
                    AccountSerial::from(u32::from(starting_serial) + i as u32)
                );
            }
        }

        #[test]
        fn proptest_modifications_always_fresh(
            num_creates in 1usize..10,
            modifications in prop::collection::vec(1u64..1000, 1..10)
        ) {
            let mut base = create_test_state();
            let mut tracker = TrackingStateAccessor::new(base.clone());

            // Create accounts and apply random modifications
            for i in 0..num_creates {
                let id = test_account_id(i as u8);
                tracker.create_new_account(id, create_empty_account_type()).unwrap();

                // Apply modifications to this account
                let mut expected_balance = 0u64;
                for &mod_amt in modifications.iter().take(i + 1) {
                    tracker.add_balance(id, Coin::new_unchecked(BitcoinAmount::from(mod_amt))).unwrap();
                    expected_balance += mod_amt;
                }

                // Verify the account in overlay has the expected balance
                let acct = tracker.get_account_state(id).unwrap().unwrap();
                prop_assert_eq!(acct.balance(), BitcoinAmount::from(expected_balance));
            }

            // Apply and verify final state
            let (writebatch, _) = tracker.finalize_as_writebatch();
            base.apply_write_batch(writebatch).unwrap();

            // Verify all accounts have correct final balances
            for i in 0..num_creates {
                let id = test_account_id(i as u8);
                let expected_balance: u64 = modifications.iter().take(i + 1).sum();
                let acct = base.get_account_state(id).unwrap().unwrap();
                prop_assert_eq!(acct.balance(), BitcoinAmount::from(expected_balance));
            }
        }

        #[test]
        fn proptest_multiple_trackers_isolated(
            creates1 in 1usize..10,
            creates2 in 1usize..10
        ) {
            let base = create_test_state();
            let starting_serial = base.get_next_serial();

            // Create two independent trackers from the SAME base (before any applies)
            let mut tracker1 = TrackingStateAccessor::new(base.clone());
            let mut tracker2 = TrackingStateAccessor::new(base.clone());

            // First tracker creates accounts
            for i in 0..creates1 {
                let id = test_account_id(i as u8);
                tracker1.create_new_account(id, create_empty_account_type()).unwrap();
            }

            // Second tracker creates accounts independently
            for i in 0..creates2 {
                let id = test_account_id((100 + i) as u8);  // Different IDs
                tracker2.create_new_account(id, create_empty_account_type()).unwrap();
            }

            // Both started from the same base, so both should allocate starting from same serial
            prop_assert_eq!(tracker1.get_next_serial(), AccountSerial::from(u32::from(starting_serial) + creates1 as u32));
            prop_assert_eq!(tracker2.get_next_serial(), AccountSerial::from(u32::from(starting_serial) + creates2 as u32));

            // Verify independence: each tracker only sees its own accounts
            for i in 0..creates1 {
                let id = test_account_id(i as u8);
                prop_assert!(tracker1.check_account_exists(id).unwrap());
                prop_assert!(!tracker2.check_account_exists(id).unwrap());
            }

            for i in 0..creates2 {
                let id = test_account_id((100 + i) as u8);
                prop_assert!(tracker2.check_account_exists(id).unwrap());
                prop_assert!(!tracker1.check_account_exists(id).unwrap());
            }
        }
    }
}
