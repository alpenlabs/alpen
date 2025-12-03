/// ExecutionStateAccessor wraps a base StateAccessor and tracks all modifications
/// using a WriteBatch (Copy-on-Write overlay) and auxiliary data.
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

use crate::{
    AccountState, EpochalState, GlobalState,
    writebatch::{ExecutionAuxiliaryData, WriteBatch},
};

/// Wraps a StateAccessor to track modifications during block execution
#[derive(Debug)]
pub struct ExecutionStateAccessor<S: StateAccessor> {
    /// Base state being wrapped (could be DB-backed, in-memory, etc.)
    base: S,

    /// Copy-on-Write overlay for modifications (consensus-critical state)
    batch: WriteBatch,

    /// Auxiliary data for database persistence (non-consensus)
    aux: ExecutionAuxiliaryData,
}

impl<S: StateAccessor<GlobalState = GlobalState, L1ViewState = EpochalState, AccountState = AccountState>>
    ExecutionStateAccessor<S>
{
    /// Create a new ExecutionStateAccessor wrapping a base state accessor
    pub fn new(base: S) -> Self {
        Self {
            base,
            batch: WriteBatch::new(),
            aux: ExecutionAuxiliaryData::default(),
        }
    }

    /// Finalize execution and extract the WriteBatch, auxiliary data, and base state
    pub fn finalize(self) -> (WriteBatch, ExecutionAuxiliaryData, S) {
        (self.batch, self.aux, self.base)
    }

    /// Get reference to the base state accessor
    pub fn base(&self) -> &S {
        &self.base
    }

    /// Get reference to the current WriteBatch
    pub fn batch(&self) -> &WriteBatch {
        &self.batch
    }

    /// Get reference to the auxiliary data
    pub fn aux(&self) -> &ExecutionAuxiliaryData {
        &self.aux
    }
}

impl<S: StateAccessor<GlobalState = GlobalState, L1ViewState = EpochalState, AccountState = AccountState>>
    StateAccessor for ExecutionStateAccessor<S>
{
    type GlobalState = GlobalState;
    type L1ViewState = EpochalState;
    type AccountState = AccountState;

    fn global(&self) -> &Self::GlobalState {
        // Check overlay first, fall through to base
        self.batch
            .global_state()
            .unwrap_or_else(|| self.base.global())
    }

    fn global_mut(&mut self) -> &mut Self::GlobalState {
        // CoW: clone from base on first write
        let base_global = self.base.global().clone();
        self.batch.global_state_mut_or_insert(base_global)
    }

    fn l1_view(&self) -> &Self::L1ViewState {
        // Check overlay first, fall through to base
        self.batch
            .epochal_state()
            .unwrap_or_else(|| self.base.l1_view())
    }

    fn l1_view_mut(&mut self) -> &mut Self::L1ViewState {
        // CoW: clone from base on first write
        let base_l1view = self.base.l1_view().clone();
        self.batch.epochal_state_mut_or_insert(base_l1view)
    }

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        // Check overlay first
        if self.batch.has_account(&id) {
            return Ok(true);
        }
        // Fall through to base
        self.base.check_account_exists(id)
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        // Check overlay first
        if let Some(acct) = self.batch.get_account(&id) {
            return Ok(Some(acct));
        }
        // Fall through to base
        self.base.get_account_state(id)
    }

    fn get_account_state_mut(
        &mut self,
        id: AccountId,
    ) -> AcctResult<Option<&mut Self::AccountState>> {
        // CoW: if not in overlay, clone from base on first write
        if !self.batch.has_account(&id) {
            if let Some(base_acct) = self.base.get_account_state(id)? {
                self.batch.insert_account(id, base_acct.clone());
            } else {
                // Account doesn't exist in base
                return Ok(None);
            }
        }

        // Return mutable reference from overlay
        Ok(self.batch.get_account_mut(&id))
    }

    fn update_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()> {
        // Verify account exists (either in overlay or base)
        if !self.batch.has_account(&id) && !self.base.check_account_exists(id)? {
            return Err(strata_acct_types::AcctError::MissingExpectedAccount(id));
        }

        // Update in overlay
        self.batch.insert_account(id, state);
        Ok(())
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        // Delegate to base for serial generation, but we need to track in overlay
        let serial = self.base.create_new_account(id, state)?;

        // Copy the newly created account into the overlay
        if let Some(new_acct) = self.base.get_account_state(id)? {
            self.batch.insert_account(id, new_acct.clone());
        }

        Ok(serial)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        // This is a lookup operation, delegate to base
        // (Serials are managed by the base state)
        // TODO: need to access writebatch as well.
        self.base.find_account_id_by_serial(serial)
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        // This is tricky - we need to compute root with overlay applied
        // For now, delegate to base and note this needs proper implementation
        // TODO: Implement proper root computation with overlay applied
        self.base.compute_state_root()
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, BitcoinAmount};
    use strata_ledger_types::{
        AccountTypeState, Coin, IAccountState, IGlobalState, StateAccessor,
    };

    use super::*;
    use crate::OLState;

    #[test]
    fn test_cow_global_state() {
        let base = OLState::new_genesis();
        let original_slot = base.global().get_cur_slot();

        let mut exec_accessor = ExecutionStateAccessor::new(base);

        // Read should return base value
        assert_eq!(exec_accessor.global().get_cur_slot(), original_slot);

        // Modify through accessor
        exec_accessor.global_mut().set_cur_slot(42);

        // Read should now return modified value
        assert_eq!(exec_accessor.global().get_cur_slot(), 42);

        // Base state should be unchanged after finalize
        let (batch, _aux, base) = exec_accessor.finalize();
        assert_eq!(base.global().get_cur_slot(), original_slot);
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
        let (batch, _aux, base) = exec_accessor.finalize();

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

        let (batch, _aux, _base) = exec_accessor.finalize();

        // Batch should be empty (no modifications)
        assert!(!batch.has_account(&acct_id));
        assert_eq!(batch.modified_accounts_count(), 0);
    }
}
