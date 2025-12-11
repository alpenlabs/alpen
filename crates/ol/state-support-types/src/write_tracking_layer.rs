//! OL state layer that stores writes into a write batch.
//!
//! This provides an `IStateAccessor` implementation that tracks all writes
//! in a `WriteBatch`, allowing them to be applied atomically or discarded.

use strata_acct_types::{AccountId, AccountSerial, AcctError, AcctResult, BitcoinAmount};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::{
    IAccountStateConstructible, IAccountStateMut, IStateAccessor, NewAccountData,
};

use crate::write_batch::WriteBatch;

/// A write-tracking state accessor that wraps a base state.
///
/// All reads check the write batch first, then fall back to the base state.
/// All writes are recorded in the write batch.
pub struct WriteTrackingState<'base, S: IStateAccessor> {
    base: &'base S,
    batch: WriteBatch<S::AccountState>,
}

impl<S: IStateAccessor> std::fmt::Debug for WriteTrackingState<'_, S>
where
    S: std::fmt::Debug,
    S::AccountState: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteTrackingState")
            .field("base", &self.base)
            .field("batch", &self.batch)
            .finish()
    }
}

impl<'base, S: IStateAccessor> WriteTrackingState<'base, S> {
    /// Creates a new write-tracking state wrapping the given base state.
    ///
    /// The global and epochal state are cloned from the base into the write batch,
    /// since they're small and always modified during block execution.
    pub fn new(base: &'base S, batch: WriteBatch<S::AccountState>) -> Self {
        Self { base, batch }
    }

    /// Returns a reference to the underlying write batch.
    pub fn batch(&self) -> &WriteBatch<S::AccountState> {
        &self.batch
    }

    /// Consumes this wrapper and returns the write batch.
    pub fn into_batch(self) -> WriteBatch<S::AccountState> {
        self.batch
    }
}

impl<'base, S: IStateAccessor> IStateAccessor for WriteTrackingState<'base, S>
where
    S::AccountState: Clone + IAccountStateConstructible + IAccountStateMut,
{
    type AccountState = S::AccountState;
    type AccountStateMut = S::AccountState; // Same type as AccountState for this layer

    // ===== Global state methods =====

    fn cur_slot(&self) -> u64 {
        self.batch.global().get_cur_slot()
    }

    fn set_cur_slot(&mut self, slot: u64) {
        self.batch.global_mut().set_cur_slot(slot);
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.batch.epochal().cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.batch.epochal_mut().set_cur_epoch(epoch);
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.batch.epochal().last_l1_blkid()
    }

    fn last_l1_height(&self) -> L1Height {
        self.batch.epochal().last_l1_height()
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        self.batch.epochal_mut().append_manifest(height, mf);
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.batch.epochal().asm_recorded_epoch()
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.batch.epochal_mut().set_asm_recorded_epoch(epoch);
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.batch.epochal().total_ledger_balance()
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.batch.epochal_mut().set_total_ledger_balance(amt);
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        // Check write batch first
        if self.batch.ledger().contains_account(&id) {
            return Ok(true);
        }
        // Fall back to base state
        self.base.check_account_exists(id)
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        // Check write batch first
        if let Some(state) = self.batch.ledger().get_account(&id) {
            return Ok(Some(state));
        }
        // Fall back to base state
        self.base.get_account_state(id)
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> AcctResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        // Copy-on-write: ensure account is in batch
        if !self.batch.ledger().contains_account(&id) {
            let account = self
                .base
                .get_account_state(id)?
                .ok_or(AcctError::MissingExpectedAccount(id))?
                .clone();
            self.batch.ledger_mut().update_account(id, account);
        }

        // Get mut ref from batch and run closure
        let account = self
            .batch
            .ledger_mut()
            .get_account_mut(&id)
            .expect("account should be in batch");
        Ok(f(account))
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        let serial = self
            .batch
            .ledger_mut()
            .create_account_from_data(id, new_acct_data);
        Ok(serial)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        // Check write batch first (for newly created accounts)
        if let Some(id) = self.batch.ledger().find_id_by_serial(serial) {
            return Ok(Some(id));
        }
        // Fall back to base state
        self.base.find_account_id_by_serial(serial)
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        // State root computation is not supported on WriteTrackingState because
        // we only have a subset of the state (modified accounts). The proper
        // state root should be computed after the batch is applied to the full state.
        Err(AcctError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::absolute;
    use strata_acct_types::{AcctError, BitcoinAmount, SYSTEM_RESERVED_ACCTS};
    use strata_asm_manifest_types::AsmManifest;
    use strata_identifiers::{
        AccountSerial, Buf32, L1BlockCommitment, L1BlockId, L1Height, WtxidsRoot,
    };
    use strata_ledger_types::{AccountTypeState, Coin, IAccountState, IAccountStateMut, IStateAccessor, NewAccountData};
    use strata_ol_state_types::{EpochalState, GlobalState, OLState};

    use super::*;
    use crate::test_utils::*;
    use crate::write_batch::WriteBatch;

    /// Helper to create a WriteBatch initialized from a base OLState.
    fn create_batch_from_state(
        state: &OLState,
    ) -> WriteBatch<<OLState as IStateAccessor>::AccountState> {
        // Create epochal state matching the base
        let epochal = EpochalState::new(
            state.total_ledger_balance(),
            state.cur_epoch(),
            L1BlockCommitment::new(
                absolute::Height::from_consensus(state.last_l1_height().into()).unwrap(),
                *state.last_l1_blkid(),
            ),
            state.asm_recorded_epoch().clone(),
        );
        let global = GlobalState::new(state.cur_slot());

        let mut batch = WriteBatch::new(global, epochal);

        // Initialize next_serial for account creation
        // The next serial is SYSTEM_RESERVED_ACCTS + number of accounts
        // For new genesis state, it would be SYSTEM_RESERVED_ACCTS
        let base_next_serial = AccountSerial::from(SYSTEM_RESERVED_ACCTS);
        batch.ledger_mut().set_next_serial(base_next_serial);

        batch
    }

    // =========================================================================
    // Copy-on-write tests
    // =========================================================================

    #[test]
    fn test_read_falls_back_to_base() {
        let account_id = test_account_id(1);
        let (base_state, serial) = setup_state_with_snark_account(
            account_id,
            1,
            BitcoinAmount::from_sat(1000),
        );

        let batch = create_batch_from_state(&base_state);
        // Need to update next_serial to account for the account we created
        let mut batch = batch;
        batch
            .ledger_mut()
            .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1));

        let tracking = WriteTrackingState::new(&base_state, batch);

        // Read should fall back to base since batch is empty
        let account = tracking.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));
    }

    #[test]
    fn test_check_account_exists_falls_back_to_base() {
        let account_id = test_account_id(1);
        let nonexistent_id = test_account_id(99);
        let (base_state, _) = setup_state_with_snark_account(
            account_id,
            1,
            BitcoinAmount::from_sat(1000),
        );

        let batch = create_batch_from_state(&base_state);
        let tracking = WriteTrackingState::new(&base_state, batch);

        assert!(tracking.check_account_exists(account_id).unwrap());
        assert!(!tracking.check_account_exists(nonexistent_id).unwrap());
    }

    #[test]
    fn test_write_copies_to_batch() {
        let account_id = test_account_id(1);
        let (base_state, _) = setup_state_with_snark_account(
            account_id,
            1,
            BitcoinAmount::from_sat(1000),
        );
        let original_balance = base_state
            .get_account_state(account_id)
            .unwrap()
            .unwrap()
            .balance();

        let mut batch = create_batch_from_state(&base_state);
        batch
            .ledger_mut()
            .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1));

        let mut tracking = WriteTrackingState::new(&base_state, batch);

        // Modify account
        tracking
            .update_account(account_id, |acct| {
                let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
                acct.add_balance(coin);
            })
            .unwrap();

        // Verify it's now in batch
        assert!(tracking.batch().ledger().contains_account(&account_id));

        // Verify the modified balance through tracking state
        let modified_account = tracking.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(modified_account.balance(), BitcoinAmount::from_sat(1500));

        // Verify base state is unchanged
        let base_account = base_state.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(base_account.balance(), original_balance);
    }

    #[test]
    fn test_read_prefers_batch_over_base() {
        let account_id = test_account_id(1);
        let (base_state, _) = setup_state_with_snark_account(
            account_id,
            1,
            BitcoinAmount::from_sat(1000),
        );

        let mut batch = create_batch_from_state(&base_state);
        batch
            .ledger_mut()
            .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1));

        let mut tracking = WriteTrackingState::new(&base_state, batch);

        // Modify the account to put it in the batch
        tracking
            .update_account(account_id, |acct| {
                let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
                acct.add_balance(coin);
            })
            .unwrap();

        // Modify again - should use batch version
        tracking
            .update_account(account_id, |acct| {
                let coin = Coin::new_unchecked(BitcoinAmount::from_sat(100));
                acct.add_balance(coin);
            })
            .unwrap();

        // Final balance should be 1000 + 500 + 100 = 1600
        let account = tracking.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1600));
    }

    // =========================================================================
    // Account creation tests
    // =========================================================================

    #[test]
    fn test_create_account_in_batch() {
        let base_state = OLState::new_genesis();
        let mut batch = create_batch_from_state(&base_state);
        batch
            .ledger_mut()
            .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS));

        let mut tracking = WriteTrackingState::new(&base_state, batch);

        let account_id = test_account_id(1);
        let snark_state = test_snark_account_state(1);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(5000),
            AccountTypeState::Snark(snark_state),
        );

        let serial = tracking.create_new_account(account_id, new_acct).unwrap();

        // Verify it's in the batch
        assert!(tracking.batch().ledger().contains_account(&account_id));

        // Verify we can retrieve it
        let account = tracking.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));

        // Verify base is unchanged
        assert!(!base_state.check_account_exists(account_id).unwrap());
    }

    #[test]
    fn test_find_account_id_by_serial_for_new_account() {
        let base_state = OLState::new_genesis();
        let mut batch = create_batch_from_state(&base_state);
        batch
            .ledger_mut()
            .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS));

        let mut tracking = WriteTrackingState::new(&base_state, batch);

        let account_id = test_account_id(1);
        let snark_state = test_snark_account_state(1);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(5000),
            AccountTypeState::Snark(snark_state),
        );

        let serial = tracking.create_new_account(account_id, new_acct).unwrap();

        // Should be able to find the account by serial
        let found_id = tracking.find_account_id_by_serial(serial).unwrap();
        assert_eq!(found_id, Some(account_id));
    }

    // =========================================================================
    // Global/epochal state tests
    // =========================================================================

    #[test]
    fn test_slot_modifications_in_batch() {
        let base_state = OLState::new_genesis();
        let batch = create_batch_from_state(&base_state);
        let mut tracking = WriteTrackingState::new(&base_state, batch);

        assert_eq!(tracking.cur_slot(), 0);

        tracking.set_cur_slot(42);

        assert_eq!(tracking.cur_slot(), 42);

        // Verify it's in the batch
        assert_eq!(tracking.batch().global().get_cur_slot(), 42);
    }

    #[test]
    fn test_epoch_modifications_in_batch() {
        let base_state = OLState::new_genesis();
        let batch = create_batch_from_state(&base_state);
        let mut tracking = WriteTrackingState::new(&base_state, batch);

        assert_eq!(tracking.cur_epoch(), 0);

        tracking.set_cur_epoch(5);

        assert_eq!(tracking.cur_epoch(), 5);

        // Verify it's in the batch
        assert_eq!(tracking.batch().epochal().cur_epoch(), 5);
    }

    #[test]
    fn test_total_ledger_balance_in_batch() {
        let base_state = OLState::new_genesis();
        let batch = create_batch_from_state(&base_state);
        let mut tracking = WriteTrackingState::new(&base_state, batch);

        tracking.set_total_ledger_balance(BitcoinAmount::from_sat(1_000_000));

        assert_eq!(
            tracking.total_ledger_balance(),
            BitcoinAmount::from_sat(1_000_000)
        );
    }

    #[test]
    fn test_manifest_append_in_batch() {
        let base_state = OLState::new_genesis();
        let batch = create_batch_from_state(&base_state);
        let mut tracking = WriteTrackingState::new(&base_state, batch);

        let height = L1Height::from(100u32);
        let l1_blkid = L1BlockId::from(Buf32::from([1u8; 32]));
        let wtxids_root = WtxidsRoot::from(Buf32::from([2u8; 32]));
        let manifest = AsmManifest::new(l1_blkid, wtxids_root, vec![]);

        tracking.append_manifest(height, manifest);

        // The manifest should be recorded in the epochal state
        // (The actual validation of this would depend on the epochal state implementation)
    }

    // =========================================================================
    // State root test
    // =========================================================================

    #[test]
    fn test_compute_state_root_returns_unsupported() {
        let base_state = OLState::new_genesis();
        let batch = create_batch_from_state(&base_state);
        let tracking = WriteTrackingState::new(&base_state, batch);

        let result = tracking.compute_state_root();
        assert!(matches!(result, Err(AcctError::Unsupported)));
    }

    // =========================================================================
    // Batch extraction tests
    // =========================================================================

    #[test]
    fn test_into_batch_returns_modifications() {
        let account_id = test_account_id(1);
        let (base_state, _) = setup_state_with_snark_account(
            account_id,
            1,
            BitcoinAmount::from_sat(1000),
        );

        let mut batch = create_batch_from_state(&base_state);
        batch
            .ledger_mut()
            .set_next_serial(AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1));

        let mut tracking = WriteTrackingState::new(&base_state, batch);

        // Make some modifications
        tracking.set_cur_slot(100);
        tracking
            .update_account(account_id, |acct| {
                let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
                acct.add_balance(coin);
            })
            .unwrap();

        // Extract the batch
        let batch = tracking.into_batch();

        // Verify modifications are in the batch
        assert_eq!(batch.global().get_cur_slot(), 100);
        assert!(batch.ledger().contains_account(&account_id));

        let account = batch.ledger().get_account(&account_id).unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1500));
    }

    #[test]
    fn test_batch_reference_accessible() {
        let base_state = OLState::new_genesis();
        let batch = create_batch_from_state(&base_state);
        let tracking = WriteTrackingState::new(&base_state, batch);

        // Should be able to access batch via reference
        let batch_ref = tracking.batch();
        assert_eq!(batch_ref.global().get_cur_slot(), 0);
    }

    // =========================================================================
    // Error handling tests
    // =========================================================================

    #[test]
    fn test_update_nonexistent_account_returns_error() {
        let base_state = OLState::new_genesis();
        let batch = create_batch_from_state(&base_state);
        let mut tracking = WriteTrackingState::new(&base_state, batch);

        let nonexistent_id = test_account_id(99);
        let result = tracking.update_account(nonexistent_id, |_acct| {});

        assert!(matches!(
            result,
            Err(AcctError::MissingExpectedAccount(_))
        ));
    }
}
