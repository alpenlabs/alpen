//! Toplevel state.

use ssz::Encode;
use strata_acct_types::{AccountId, AccountSerial, AcctError, AcctResult, BitcoinAmount, Mmr64};
use strata_asm_manifest_types::AsmManifest;
use strata_crypto::hash::raw;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::*;
use strata_merkle::CompactMmr64;
use strata_ol_params::OLParams;

use crate::{
    IStateBatchApplicable, WriteBatch,
    ssz_generated::ssz::state::{
        EpochalState, GlobalState, OLAccountState, OLState, TsnlLedgerAccountsTable,
    },
};

impl OLState {
    /// Creates initial OL state from genesis parameters.
    pub fn from_genesis_params(params: &OLParams) -> AcctResult<Self> {
        let checkpointed_epoch = params.checkpointed_epoch();
        let manifests_mmr = Mmr64::from_generic(&CompactMmr64::new(64));

        let ledger = TsnlLedgerAccountsTable::from_genesis_account_params(&params.accounts)?;
        let total_ledger_funds = ledger.calculate_total_funds();

        let global = GlobalState::new(params.header.slot);
        let manifests_mmr_offset = params.last_l1_block.height_u64() + 1;
        let epoch = EpochalState::new(
            total_ledger_funds,
            params.header.epoch,
            params.last_l1_block,
            checkpointed_epoch,
            manifests_mmr,
            manifests_mmr_offset,
        );
        Ok(Self {
            epoch,
            global,
            ledger,
        })
    }

    /// Checks that a batch can be applied safely.
    ///
    /// This checks:
    /// * new accounts being created have correct serials
    /// * supposedly-existing accounts being updated are real
    ///
    /// This function failing probably indicates the write batch was not
    /// intended for the state we're trying to apply it to, or some bug with how
    /// we're constructing write batches.
    pub fn check_write_batch_safe(&self, batch: &WriteBatch<OLAccountState>) -> AcctResult<()> {
        // Check serial ordering.
        let mut next_serial = self.ledger.next_avail_serial();
        for (serial, id) in batch.ledger().iter_new_accounts() {
            let state = batch
                .ledger()
                .get_account(id)
                .expect("state: batch with dangling serial entry");

            // Check that the entry is consistent.
            if state.serial() != serial {
                return Err(AcctError::AccountSerialInconsistent(
                    *id,
                    state.serial(),
                    serial,
                ));
            }

            // Check that it works.
            if serial != next_serial {
                return Err(AcctError::SerialSequence(serial, next_serial));
            }

            // Make sure that the account doesn't already exist.
            if self.ledger.get_account_state(id).is_some() {
                return Err(AcctError::CreateExistingAccount(*id));
            }

            // Update next serial as if we added the account.
            next_serial = next_serial.incr();
        }

        // Now check that all existing accounts really exist.
        for (id, state) in batch.ledger().iter_accounts() {
            // At this point we know that if the serial is greater than the
            // current highest serial then it doesn't exist yet, which we've
            // already checked for.
            if state.serial() >= self.ledger.next_avail_serial() {
                continue;
            }

            // Now make sure it exists.
            if self.ledger.get_account_state(id).is_none() {
                return Err(AcctError::UpdateNonexistentAccount(*id));
            }
        }

        Ok(())
    }

    /// Applies a write batch to this state.
    ///
    /// This updates the global state, epochal state, and ledger accounts
    /// with the modifications from the batch.
    ///
    /// If this returns an error then the state is left unmodified.
    pub fn apply_write_batch(&mut self, batch: WriteBatch<OLAccountState>) -> AcctResult<()> {
        // Safety check first so we can use `.expect`.
        self.check_write_batch_safe(&batch)?;
        let (global, epochal, ledger) = batch.into_parts();

        // Separate new accounts from updates.
        let (new_accounts, updated_accounts) = ledger.into_new_and_updated();

        // Create new accounts.
        for (account_id, account_state) in new_accounts {
            self.ledger
                .create_account(account_id, account_state)
                .expect("state: failed to create account");
        }

        // Update existing accounts.
        for (account_id, account_state) in updated_accounts {
            let existing = self
                .ledger
                .get_account_state_mut(&account_id)
                .expect("state: missing expected account");
            *existing = account_state;
        }

        // Finally, update global and epochal state.
        self.global = global;
        self.epoch = epochal;

        Ok(())
    }
}

impl IStateAccessor for OLState {
    type AccountState = OLAccountState;
    type AccountStateMut = OLAccountState;

    // ===== Global state methods =====

    fn cur_slot(&self) -> u64 {
        self.global.get_cur_slot()
    }

    fn set_cur_slot(&mut self, slot: u64) {
        self.global.set_cur_slot(slot);
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.epoch.cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.epoch.set_cur_epoch(epoch);
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.epoch.last_l1_blkid()
    }

    fn last_l1_height(&self) -> L1Height {
        self.epoch.last_l1_height()
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        self.epoch.append_manifest(height, mf);
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.epoch.asm_recorded_epoch()
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.epoch.set_asm_recorded_epoch(epoch);
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.epoch.total_ledger_balance()
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.epoch.set_total_ledger_balance(amt);
    }

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        self.epoch.asm_manifests_mmr()
    }

    fn asm_manifests_mmr_offset(&self) -> u64 {
        self.epoch.manifests_mmr_offset()
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        Ok(self.ledger.get_account_state(&id).is_some())
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        Ok(self.ledger.get_account_state(&id))
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> AcctResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        let acct = self
            .ledger
            .get_account_state_mut(&id)
            .ok_or(AcctError::MissingExpectedAccount(id))?;
        Ok(f(acct))
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        let serial = self.ledger.next_avail_serial();
        let account = OLAccountState::new_with_serial(new_acct_data, serial);
        self.ledger.create_account(id, account)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        Ok(self.ledger.get_serial_acct_id(serial).copied())
    }

    fn next_account_serial(&self) -> AccountSerial {
        self.ledger.next_avail_serial()
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        // Compute the state root by hashing the SSZ encoding of the state
        let encoded = self.as_ssz_bytes();
        let hash = raw(&encoded);
        Ok(hash)
    }
}

impl IStateBatchApplicable for OLState {
    fn apply_write_batch(&mut self, batch: WriteBatch<Self::AccountState>) -> AcctResult<()> {
        // Delegate to the inherent method
        OLState::apply_write_batch(self, batch)
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_ledger_types::{AccountTypeState, IAccountState, NewAccountData};
    use strata_predicate::PredicateKey;

    use super::*;
    use crate::{OLAccountTypeState, OLSnarkAccountState, test_utils::create_test_genesis_state};

    fn test_account_id(seed: u8) -> AccountId {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        AccountId::from(bytes)
    }

    #[test]
    fn test_apply_batch_updates_global_state() {
        let mut state = create_test_genesis_state();
        let mut batch = WriteBatch::new_from_state(&state);

        // Modify slot in batch.
        batch.global_mut().set_cur_slot(42);

        state.apply_write_batch(batch).unwrap();

        assert_eq!(state.cur_slot(), 42);
    }

    #[test]
    fn test_apply_batch_updates_epochal_state() {
        let mut state = create_test_genesis_state();
        let mut batch = WriteBatch::new_from_state(&state);

        // Modify epoch in batch.
        batch.epochal_mut().set_cur_epoch(5);

        state.apply_write_batch(batch).unwrap();

        assert_eq!(state.cur_epoch(), 5);
    }

    #[test]
    fn test_apply_batch_creates_new_account() {
        let mut state = create_test_genesis_state();
        let account_id = test_account_id(1);
        let mut batch = WriteBatch::new_from_state(&state);

        // Create a new account in the batch.
        let snark_state =
            OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), [0u8; 32].into());
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(1000),
            AccountTypeState::Snark(snark_state),
        );
        let serial = state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        // Apply the batch.
        state.apply_write_batch(batch).unwrap();

        // Verify account exists and has correct balance.
        assert!(state.check_account_exists(account_id).unwrap());
        let account = state.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));
        assert_eq!(account.serial(), serial);
    }

    #[test]
    fn test_apply_batch_updates_existing_account() {
        let mut state = create_test_genesis_state();
        let account_id = test_account_id(1);

        // Create an account directly in state.
        let snark_state =
            OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), [0u8; 32].into());
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(1000),
            AccountTypeState::Snark(snark_state),
        );
        let serial = state.create_new_account(account_id, new_acct).unwrap();

        // Create a batch that updates the account.
        let mut batch = WriteBatch::new_from_state(&state);
        let snark_state_updated =
            OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), [1u8; 32].into());
        let updated_account = OLAccountState::new(
            serial,
            BitcoinAmount::from_sat(2000),
            OLAccountTypeState::Snark(snark_state_updated),
        );
        batch
            .ledger_mut()
            .update_account(account_id, updated_account);

        // Apply the batch.
        state.apply_write_batch(batch).unwrap();

        // Verify account was updated.
        let account = state.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(2000));
    }

    #[test]
    fn test_apply_batch_multiple_changes() {
        let mut state = create_test_genesis_state();
        let account_id_1 = test_account_id(1);
        let account_id_2 = test_account_id(2);

        let mut batch = WriteBatch::new_from_state(&state);

        // Modify global state.
        batch.global_mut().set_cur_slot(100);

        // Modify epochal state.
        batch.epochal_mut().set_cur_epoch(10);

        // Create two new accounts.
        let snark_state_1 =
            OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), [0u8; 32].into());
        let new_acct_1 = NewAccountData::new(
            BitcoinAmount::from_sat(1000),
            AccountTypeState::Snark(snark_state_1),
        );
        let serial_1 = state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id_1, new_acct_1, serial_1);

        let snark_state_2 =
            OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), [1u8; 32].into());
        let new_acct_2 = NewAccountData::new(
            BitcoinAmount::from_sat(2000),
            AccountTypeState::Snark(snark_state_2),
        );
        let serial_2 = AccountSerial::from(serial_1.inner() + 1);
        batch
            .ledger_mut()
            .create_account_from_data(account_id_2, new_acct_2, serial_2);

        // Apply the batch.
        state.apply_write_batch(batch).unwrap();

        // Verify all changes applied.
        assert_eq!(state.cur_slot(), 100);
        assert_eq!(state.cur_epoch(), 10);
        assert!(state.check_account_exists(account_id_1).unwrap());
        assert!(state.check_account_exists(account_id_2).unwrap());

        let account_1 = state.get_account_state(account_id_1).unwrap().unwrap();
        assert_eq!(account_1.balance(), BitcoinAmount::from_sat(1000));

        let account_2 = state.get_account_state(account_id_2).unwrap().unwrap();
        assert_eq!(account_2.balance(), BitcoinAmount::from_sat(2000));
    }

    #[test]
    fn test_apply_batch_creates_and_updates_accounts() {
        // Actually now that I think about it, this test is kinda a duplicate.

        let mut state = create_test_genesis_state();
        let existing_id = test_account_id(1);
        let new_id = test_account_id(2);

        // Create an existing account in state first.
        let snark_state =
            OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), [0u8; 32].into());
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(1000),
            AccountTypeState::Snark(snark_state),
        );
        let existing_serial = state.create_new_account(existing_id, new_acct).unwrap();

        // Create a batch that both updates existing and creates new.
        let mut batch = WriteBatch::new_from_state(&state);

        // Update the existing account.
        let updated_snark =
            OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), [1u8; 32].into());
        let updated_account = OLAccountState::new(
            existing_serial,
            BitcoinAmount::from_sat(5000),
            OLAccountTypeState::Snark(updated_snark),
        );
        batch
            .ledger_mut()
            .update_account(existing_id, updated_account);

        // Create a new account.
        let new_snark =
            OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), [2u8; 32].into());
        let new_acct_data = NewAccountData::new(
            BitcoinAmount::from_sat(3000),
            AccountTypeState::Snark(new_snark),
        );
        let new_serial = state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(new_id, new_acct_data, new_serial);

        // Apply the batch.
        state.apply_write_batch(batch).unwrap();

        // Verify existing account was updated.
        let existing_account = state.get_account_state(existing_id).unwrap().unwrap();
        assert_eq!(existing_account.balance(), BitcoinAmount::from_sat(5000));
        assert_eq!(existing_account.serial(), existing_serial);

        // Verify new account was created.
        assert!(state.check_account_exists(new_id).unwrap());
        let new_account = state.get_account_state(new_id).unwrap().unwrap();
        assert_eq!(new_account.balance(), BitcoinAmount::from_sat(3000));
        assert_eq!(new_account.serial(), new_serial);
    }

    mod ol_state {
        use strata_test_utils_ssz::ssz_proptest;

        use super::*;
        use crate::test_utils::ol_state_strategy;

        ssz_proptest!(OLState, ol_state_strategy());
    }
}
