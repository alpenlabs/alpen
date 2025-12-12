/// ExecutionStateAccessor wraps a base StateAccessor and tracks all modifications
/// using a WriteBatch (Copy-on-Write overlay).
///
/// This enables:
/// - In-memory modification tracking over a potentially DB-backed base state
/// - Fast lookups for modified state (from memory, not DB)
/// - Separation of consensus state (WriteBatch) from indexing data (aux)
///
/// The CoW mechanism inherently tracks which accounts were modified - any account
/// in the WriteBatch's modified_accounts map was accessed mutably.
use strata_acct_types::{AccountId, AccountSerial, AcctResult};
use strata_identifiers::Buf32;
use strata_ledger_types::{AccountTypeState, StateAccessor};

use crate::writebatch::WriteBatch;

/// Wraps a StateAccessor to track modifications during block execution
pub struct ExecutionStateAccessor<S: StateAccessor> {
    /// Base state being wrapped (could be DB-backed, in-memory, etc.)
    base: S,

    /// Copy-on-Write overlay for modifications (consensus-critical state)
    write_batch: WriteBatch<S>,
}

impl<S> std::fmt::Debug for ExecutionStateAccessor<S>
where
    S: StateAccessor + std::fmt::Debug,
    WriteBatch<S>: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionStateAccessor")
            .field("base", &self.base)
            .field("write_batch", &self.write_batch)
            .finish()
    }
}

impl<S: StateAccessor> ExecutionStateAccessor<S> {
    /// Create a new ExecutionStateAccessor wrapping a base state accessor
    pub fn new(base: S) -> Self {
        Self {
            base,
            write_batch: WriteBatch::new(),
        }
    }

    /// Finalize execution and extract the WriteBatch and base state
    pub fn finalize(self) -> (WriteBatch<S>, S) {
        (self.write_batch, self.base)
    }

    /// Get reference to the base state accessor
    pub fn base(&self) -> &S {
        &self.base
    }

    /// Get reference to the current WriteBatch
    pub fn write_batch(&self) -> &WriteBatch<S> {
        &self.write_batch
    }
}

impl<S: StateAccessor> StateAccessor for ExecutionStateAccessor<S> {
    type GlobalState = S::GlobalState;
    type L1ViewState = S::L1ViewState;
    type AccountState = S::AccountState;

    fn global(&self) -> &Self::GlobalState {
        // Check overlay first, fall through to base
        self.write_batch
            .global_state()
            .unwrap_or_else(|| self.base.global())
    }

    fn global_mut(&mut self) -> &mut Self::GlobalState {
        let base_global = self.base.global();
        self.write_batch.global_state_mut_or_insert(base_global)
    }

    fn l1_view(&self) -> &Self::L1ViewState {
        // Check overlay first, fall through to base
        self.write_batch
            .epochal_state()
            .unwrap_or_else(|| self.base.l1_view())
    }

    fn l1_view_mut(&mut self) -> &mut Self::L1ViewState {
        let base_l1view = self.base.l1_view();
        self.write_batch.epochal_state_mut_or_insert(base_l1view)
    }

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        // Check overlay first
        if self.write_batch.has_account(&id) {
            return Ok(true);
        }
        // Fall through to base
        self.base.check_account_exists(id)
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        // Check overlay first
        if let Some(acct) = self.write_batch.get_account(&id) {
            return Ok(Some(acct));
        }
        // Fall through to base
        self.base.get_account_state(id)
    }

    fn get_account_state_mut(
        &mut self,
        id: AccountId,
    ) -> AcctResult<Option<&mut Self::AccountState>> {
        if !self.write_batch.has_account(&id) {
            if let Some(base_acct) = self.base.get_account_state(id)? {
                self.write_batch.insert_account(id, base_acct.clone());
            } else {
                // Account doesn't exist in base
                return Ok(None);
            }
        }

        // Return mutable reference from overlay
        Ok(self.write_batch.get_account_mut(&id))
    }

    fn update_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()> {
        // Verify account exists (either in overlay or base)
        if !self.write_batch.has_account(&id) && !self.base.check_account_exists(id)? {
            return Err(strata_acct_types::AcctError::MissingExpectedAccount(id));
        }

        // Update in overlay
        self.write_batch.insert_account(id, state);
        Ok(())
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        // Delegate to base for serial generation, but we need to track in overlay
        // FIXME: This feels weird though, now the new account is in both places. but maybe its
        // fine.
        let serial = self.base.create_new_account(id, state)?;

        // Copy the newly created account into the overlay
        if let Some(new_acct) = self.base.get_account_state(id)? {
            self.write_batch.insert_account(id, new_acct.clone());
        } else {
            // TODO: should error out id not found? because we just created a new one above.
        }

        Ok(serial)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        // This is a lookup operation, delegate to base
        // (Serials are managed by the base state)
        // TODO: need to access writebatch first and then only base.
        self.base.find_account_id_by_serial(serial)
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        // This is tricky - we need to compute root with overlay applied
        // For now, delegate to base and note this needs proper implementation
        // TODO: Implement proper root computation with overlay applied
        self.base.compute_state_root() // this is not correct! It misses the items in writebatch
        // overlay
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, BitcoinAmount};
    use strata_ledger_types::{
        AccountTypeState, Coin, IAccountState, IGlobalState, IL1ViewState, StateAccessor,
    };

    use super::*;
    use crate::OLState;

    // Test helpers
    fn create_test_account(base: &mut OLState, id_byte: u8) -> AccountId {
        let mut buf = [0u8; 32];
        buf[0] = id_byte;
        let acct_id = AccountId::from(buf);
        base.create_new_account(acct_id, AccountTypeState::Empty)
            .unwrap();
        acct_id
    }

    fn add_balance(accessor: &mut ExecutionStateAccessor<OLState>, id: AccountId, amount: u64) {
        accessor
            .get_account_state_mut(id)
            .unwrap()
            .unwrap()
            .add_balance(Coin::new_unchecked(BitcoinAmount::from(amount)));
    }

    #[test]
    fn test_cow_global_state() {
        let base = OLState::new_genesis();
        let original_global = base.global().clone();

        let mut exec_accessor = ExecutionStateAccessor::new(base);

        // Read should return base value
        assert_eq!(*exec_accessor.global(), original_global);

        // Modify through accessor
        exec_accessor.global_mut().set_cur_slot(42);

        // Read should now return modified value
        assert_eq!(exec_accessor.global().get_cur_slot(), 42);

        // Base state should be unchanged after finalize
        let (batch, base) = exec_accessor.finalize();
        assert_eq!(*base.global(), original_global);
        assert!(batch.global_state().is_some());
    }

    #[test]
    fn test_cow_account_state() {
        let mut base = OLState::new_genesis();

        // Create an account in base
        let mut buf = [0u8; 32];
        buf[0] = 1;
        let acct_id = AccountId::from(buf);
        base.create_new_account(acct_id, AccountTypeState::Empty)
            .unwrap();

        let mut exec_accessor = ExecutionStateAccessor::new(base);

        // Account should exist
        assert!(exec_accessor.check_account_exists(acct_id).unwrap());

        // Get mutable reference should trigger CoW
        let acct = exec_accessor
            .get_account_state_mut(acct_id)
            .unwrap()
            .unwrap();
        acct.add_balance(Coin::new_unchecked(BitcoinAmount::from(100)));

        // Changes should be visible
        let balance = exec_accessor
            .get_account_state(acct_id)
            .unwrap()
            .unwrap()
            .balance();
        assert_eq!(balance, BitcoinAmount::from(100));

        // Finalize and check
        let (batch, base) = exec_accessor.finalize();

        // Base should have original balance (0)
        let base_balance = base.get_account_state(acct_id).unwrap().unwrap().balance();
        assert_eq!(base_balance, BitcoinAmount::from(0));

        // Batch should contain modified account
        assert!(batch.has_account(&acct_id));
        assert_eq!(
            batch.get_account(&acct_id).unwrap().balance(),
            BitcoinAmount::from(100)
        );
    }

    #[test]
    fn test_read_through_without_modification() {
        let mut base = OLState::new_genesis();

        // Create an account in base
        let mut buf = [0u8; 32];
        buf[0] = 1;
        let acct_id = AccountId::from(buf);
        base.create_new_account(acct_id, AccountTypeState::Empty)
            .unwrap();

        let exec_accessor = ExecutionStateAccessor::new(base);

        // Read should work without triggering CoW
        assert!(exec_accessor.check_account_exists(acct_id).unwrap());
        let acct = exec_accessor.get_account_state(acct_id).unwrap().unwrap();
        assert_eq!(acct.balance(), BitcoinAmount::from(0));

        let (batch, _base) = exec_accessor.finalize();

        // Batch should be empty (no modifications)
        assert!(!batch.has_account(&acct_id));
        assert_eq!(batch.modified_accounts_count(), 0);
    }

    #[test]
    fn test_cow_clones_only_once_per_entity() {
        let mut base = OLState::new_genesis();
        let acct_id = create_test_account(&mut base, 1);

        let mut accessor = ExecutionStateAccessor::new(base);

        // Multiple mutable accesses
        for i in 1..=5 {
            add_balance(&mut accessor, acct_id, i * 10);
        }

        let (batch, _) = accessor.finalize();

        assert_eq!(batch.modified_accounts_count(), 1);
        assert_eq!(
            batch.get_account(&acct_id).unwrap().balance(),
            BitcoinAmount::from(150)
        );
    }

    #[test]
    fn test_overlay_precedence_on_reads() {
        let mut base = OLState::new_genesis();
        let acct_id = create_test_account(&mut base, 1);

        let mut accessor = ExecutionStateAccessor::new(base);

        add_balance(&mut accessor, acct_id, 100);

        // Read should return overlay version
        let balance = accessor
            .get_account_state(acct_id)
            .unwrap()
            .unwrap()
            .balance();
        assert_eq!(balance, BitcoinAmount::from(100));

        let (batch, base_after) = accessor.finalize();

        // Base unchanged, overlay has modification
        assert_eq!(
            base_after
                .get_account_state(acct_id)
                .unwrap()
                .unwrap()
                .balance(),
            BitcoinAmount::from(0)
        );
        assert_eq!(
            batch.get_account(&acct_id).unwrap().balance(),
            BitcoinAmount::from(100)
        );
    }

    #[test]
    fn test_multiple_accounts_independent() {
        let mut base = OLState::new_genesis();
        let ids: Vec<_> = (1..=3).map(|i| create_test_account(&mut base, i)).collect();

        let mut accessor = ExecutionStateAccessor::new(base);

        for (i, id) in ids.iter().enumerate() {
            add_balance(&mut accessor, *id, (i as u64 + 1) * 100);
        }

        let (batch, _) = accessor.finalize();

        assert_eq!(batch.modified_accounts_count(), 3);
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(
                batch.get_account(id).unwrap().balance(),
                BitcoinAmount::from((i as u64 + 1) * 100)
            );
        }
    }

    #[test]
    fn test_empty_accessor_empty_batch() {
        let base = OLState::new_genesis();
        let accessor = ExecutionStateAccessor::new(base);
        let (batch, _) = accessor.finalize();

        assert_eq!(batch.modified_accounts_count(), 0);
        assert!(batch.global_state().is_none());
        assert!(batch.epochal_state().is_none());
    }

    #[test]
    fn test_global_state_cow_reuse() {
        let base = OLState::new_genesis();
        let mut accessor = ExecutionStateAccessor::new(base);

        for slot in [10, 20, 30] {
            accessor.global_mut().set_cur_slot(slot);
        }

        let (batch, _) = accessor.finalize();
        assert_eq!(batch.global_state().unwrap().get_cur_slot(), 30);
    }

    #[test]
    fn test_l1_view_cow_reuse() {
        let base = OLState::new_genesis();
        let mut accessor = ExecutionStateAccessor::new(base);

        for epoch in [1, 2, 3] {
            accessor.l1_view_mut().set_cur_epoch(epoch);
        }

        let (batch, _) = accessor.finalize();
        // Verify final epoch was set (can't read directly without getter)
        assert!(batch.epochal_state().is_some());
    }

    #[test]
    fn test_nonexistent_account_returns_none() {
        let base = OLState::new_genesis();
        let mut accessor = ExecutionStateAccessor::new(base);
        let fake_id = AccountId::from([99u8; 32]);

        assert!(!accessor.check_account_exists(fake_id).unwrap());
        assert!(accessor.get_account_state(fake_id).unwrap().is_none());
        assert!(accessor.get_account_state_mut(fake_id).unwrap().is_none());

        let (batch, _) = accessor.finalize();
        assert_eq!(batch.modified_accounts_count(), 0);
    }
}
