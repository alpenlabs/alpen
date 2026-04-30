//! Toplevel state.

use strata_acct_types::{AccountId, AccountSerial, Mmr64, SYSTEM_RESERVED_ACCTS};
use strata_ledger_types::{NewAccountData, StateError, StateResult};
use strata_merkle::CompactMmr64;
use strata_ol_params::OLParams;

use crate::{
    OLAccountTypeState, OLSnarkAccountState, WriteBatch,
    ssz_generated::ssz::state::{
        EpochalState, GlobalState, OLAccountState, OLState, TsnlLedgerAccountsTable,
    },
};

impl OLState {
    /// Creates initial OL state from genesis parameters.
    pub fn from_genesis_params(params: &OLParams) -> StateResult<Self> {
        let checkpointed_epoch = params.checkpointed_epoch();
        let manifests_mmr = Mmr64::from_generic(&CompactMmr64::new(64));

        let mut next_serial = AccountSerial::new(SYSTEM_RESERVED_ACCTS);
        let mut ledger = TsnlLedgerAccountsTable::new_empty();

        // Create initial snark accounts.
        for (id, acct_params) in &params.accounts {
            // Claim the serial.
            let serial = next_serial;
            next_serial = next_serial.incr();

            // Then just assemble the rest of the account data.
            let snark_state = OLSnarkAccountState::new_fresh(
                acct_params.predicate.clone(),
                acct_params.inner_state,
            );

            let state = OLAccountState::new(
                serial,
                acct_params.balance,
                OLAccountTypeState::Snark(snark_state),
            );

            ledger.create_account(*id, state)?;
        }

        let total_ledger_funds = ledger.calculate_total_funds();

        let global = GlobalState::new(params.header.slot, next_serial);
        let manifests_mmr_offset = params.last_l1_block.height() as u64 + 1;
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

    pub fn global_state(&self) -> &GlobalState {
        &self.global
    }

    pub fn epoch_state(&self) -> &EpochalState {
        &self.epoch
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
    pub fn check_write_batch_safe(&self, batch: &WriteBatch<OLAccountState>) -> StateResult<()> {
        // Check serial ordering.
        let mut next_serial = self.global.get_next_avail_serial();
        for (serial, id) in batch.ledger().iter_new_accounts() {
            let state = batch
                .ledger()
                .get_account(id)
                .expect("state: batch with dangling serial entry");

            // Check that the entry is consistent.
            if state.serial() != serial {
                return Err(StateError::AcctSerialInconsistent {
                    id: *id,
                    in_acct: state.serial(),
                    in_table: serial,
                });
            }

            // Check that it works.
            if serial != next_serial {
                return Err(StateError::NextSerialSequence {
                    cur: serial,
                    new: next_serial,
                });
            }

            // Make sure that the account doesn't already exist.
            if self.ledger.get_account_state(id).is_some() {
                return Err(StateError::AccountExists(*id));
            }

            // Update next serial as if we added the account.
            next_serial = next_serial.incr();
        }

        // Now check that all existing accounts really exist.
        for (id, state) in batch.ledger().iter_accounts() {
            // At this point we know that if the serial is greater than the
            // current highest serial then it doesn't exist yet, which we've
            // already checked for.
            if state.serial() >= self.global.get_next_avail_serial() {
                continue;
            }

            // Now make sure it exists.
            if self.ledger.get_account_state(id).is_none() {
                return Err(StateError::AccountSanityCheckFail(*id));
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
    pub fn apply_write_batch(&mut self, batch: WriteBatch<OLAccountState>) -> StateResult<()> {
        // Safety check first so we can use `.expect`.
        self.check_write_batch_safe(&batch)?;
        let (global_writes, epochal_writes, ledger) = batch.into_parts();

        // Separate new accounts from updates.
        let (new_accounts, updated_accounts) = ledger.into_new_and_updated();

        // Create new accounts and update the serial counter.
        let mut num_new_accounts = 0usize;
        for (account_id, account_state) in new_accounts {
            self.ledger
                .create_account(account_id, account_state)
                .expect("state: failed to create account");
            num_new_accounts += 1;
        }
        if num_new_accounts > 0 {
            let next_serial = self.global.get_next_avail_serial();
            let new_serial =
                AccountSerial::from(next_serial.into_inner() + num_new_accounts as u32);
            self.global.set_next_avail_serial(new_serial);
        }

        // Update existing accounts.
        for (account_id, account_state) in updated_accounts {
            let existing = self
                .ledger
                .get_account_state_mut(&account_id)
                .expect("state: missing expected account");
            *existing = account_state;
        }

        // Apply global state writes.
        if let Some(slot) = global_writes.cur_slot {
            self.global.set_cur_slot(slot);
        }

        // Apply epochal state writes.
        if let Some(epoch) = epochal_writes.cur_epoch {
            self.epoch.set_cur_epoch(epoch);
        }

        if let Some(blkid) = epochal_writes.last_l1_blkid {
            // We need to set both blkid and height together via last_l1_block.
            // For now, set them individually.
            let height = epochal_writes
                .last_l1_height
                .unwrap_or_else(|| self.epoch.last_l1_height());
            self.epoch.last_l1_block = strata_identifiers::L1BlockCommitment::new(height, blkid);
        } else if let Some(height) = epochal_writes.last_l1_height {
            self.epoch.last_l1_block =
                strata_identifiers::L1BlockCommitment::new(height, *self.epoch.last_l1_blkid());
        }

        if let Some(epoch) = epochal_writes.asm_recorded_epoch {
            self.epoch.set_asm_recorded_epoch(epoch);
        }

        if let Some(amt) = epochal_writes.total_ledger_balance {
            self.epoch.set_total_ledger_balance(amt);
        }

        if let Some(mmr) = epochal_writes.asm_manifests_mmr {
            self.epoch.manifests_mmr = mmr;
        }

        Ok(())
    }

    /// Creates a new account.
    ///
    /// # Panics
    ///
    /// If the serial isn't the expected next serial.
    pub fn create_new_account(
        &mut self,
        id: AccountId,
        serial: AccountSerial,
        new_acct_data: NewAccountData,
    ) -> StateResult<()> {
        // Sanity check serials.
        let exp_serial = self.global.get_next_avail_serial();
        debug_assert_eq!(exp_serial, serial, "state: inconsistent serials");

        self.ledger.create_new_account(id, serial, new_acct_data)?;

        // FIXME(STR-3227): conversions
        self.global.next_avail_serial = serial.incr().into_inner() as u64;

        Ok(())
    }

    #[cfg(test)]
    pub fn next_account_serial(&self) -> AccountSerial {
        self.global.get_next_avail_serial()
    }

    #[cfg(test)]
    #[deprecated(note = "use `next_account_serial`")]
    pub fn next_avail_serial(&self) -> AccountSerial {
        self.next_account_serial()
    }

    pub fn get_account_state(&self, id: &AccountId) -> Option<&OLAccountState> {
        self.ledger.get_account_state(id)
    }

    #[cfg(test)]
    pub fn check_account_exists(&self, id: &AccountId) -> Result<(), StateError> {
        self.ledger
            .get_account_state(id)
            .map(|_| ())
            .ok_or(StateError::MissingAccount(*id))
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{BitcoinAmount, SYSTEM_RESERVED_ACCTS};
    use strata_ledger_types::{IAccountState, NewAccountData};
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
        let mut batch = WriteBatch::default();

        // Modify slot in batch.
        batch.global_writes_mut().cur_slot = Some(42);

        state.apply_write_batch(batch).unwrap();

        assert_eq!(state.global.cur_slot, 42);
    }

    #[test]
    fn test_apply_batch_updates_epochal_state() {
        let mut state = create_test_genesis_state();
        let mut batch = WriteBatch::default();

        // Modify epoch in batch.
        batch.epochal_writes_mut().cur_epoch = Some(5);

        state.apply_write_batch(batch).unwrap();

        assert_eq!(state.epoch.cur_epoch, 5);
    }

    #[test]
    fn test_apply_batch_creates_new_account() {
        let mut state = create_test_genesis_state();
        let account_id = test_account_id(1);
        let mut batch = WriteBatch::default();

        // Create a new account in the batch.
        let new_acct = NewAccountData::new_snark(
            BitcoinAmount::from_sat(1000),
            PredicateKey::always_accept(),
            [0u8; 32].into(),
        );

        let serial = state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        // Apply the batch.
        state.apply_write_batch(batch).unwrap();

        // Verify account exists and has correct balance.
        assert!(state.get_account_state(&account_id).is_some());
        let account = state.get_account_state(&account_id).unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));
        assert_eq!(account.serial(), serial);
    }

    #[test]
    fn test_apply_batch_updates_existing_account() {
        let mut state = create_test_genesis_state();
        let account_id = test_account_id(1);
        let serial = AccountSerial::new(SYSTEM_RESERVED_ACCTS);

        // Create an account directly in state.
        let new_acct = NewAccountData::new_snark(
            BitcoinAmount::from_sat(1000),
            PredicateKey::always_accept(),
            [0u8; 32].into(),
        );
        state
            .ledger
            .create_new_account(account_id, serial, new_acct)
            .unwrap();

        // Create a batch that updates the account.
        let mut batch = WriteBatch::default();
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
        let account = state.get_account_state(&account_id).unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(2000));
    }

    #[test]
    fn test_apply_batch_multiple_changes() {
        let mut state = create_test_genesis_state();
        let account_id_1 = test_account_id(1);
        let account_id_2 = test_account_id(2);

        let mut batch = WriteBatch::default();

        // Modify global state.
        batch.global_writes_mut().cur_slot = Some(100);

        // Modify epochal state.
        batch.epochal_writes_mut().cur_epoch = Some(10);

        // Create two new accounts.
        let new_acct_1 = NewAccountData::new_snark(
            BitcoinAmount::from_sat(1000),
            PredicateKey::always_accept(),
            [0u8; 32].into(),
        );
        let serial_1 = state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id_1, new_acct_1, serial_1);

        let new_acct_2 = NewAccountData::new_snark(
            BitcoinAmount::from_sat(2000),
            PredicateKey::always_accept(),
            [1u8; 32].into(),
        );
        let serial_2 = AccountSerial::from(serial_1.inner() + 1);
        batch
            .ledger_mut()
            .create_account_from_data(account_id_2, new_acct_2, serial_2);

        // Apply the batch.
        state.apply_write_batch(batch).unwrap();

        // Verify all changes applied.
        assert_eq!(state.global.cur_slot, 100);
        assert_eq!(state.epoch.cur_epoch, 10);
        assert!(state.get_account_state(&account_id_1).is_some());
        assert!(state.get_account_state(&account_id_2).is_some());

        let account_1 = state.get_account_state(&account_id_1).unwrap();
        assert_eq!(account_1.balance(), BitcoinAmount::from_sat(1000));

        let account_2 = state.get_account_state(&account_id_2).unwrap();
        assert_eq!(account_2.balance(), BitcoinAmount::from_sat(2000));
    }

    #[test]
    fn test_apply_batch_creates_and_updates_accounts() {
        // Actually now that I think about it, this test is kinda a duplicate.

        let mut state = create_test_genesis_state();
        let existing_id = test_account_id(1);
        let existing_serial = AccountSerial::new(SYSTEM_RESERVED_ACCTS);
        let new_id = test_account_id(2);

        // Create an existing account in state first.
        let new_acct = NewAccountData::new_snark(
            BitcoinAmount::from_sat(1000),
            PredicateKey::always_accept(),
            [0u8; 32].into(),
        );
        state
            .ledger
            .create_new_account(existing_id, existing_serial, new_acct)
            .expect("test: create_new_account");

        // Create a batch that both updates existing and creates new.
        let mut batch = WriteBatch::default();

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
        let new_acct_data = NewAccountData::new_snark(
            BitcoinAmount::from_sat(3000),
            PredicateKey::always_accept(),
            [2u8; 32].into(),
        );
        let new_serial = state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(new_id, new_acct_data, new_serial);

        // Apply the batch.
        state.apply_write_batch(batch).unwrap();

        // Verify existing account was updated.
        let existing_account = state.get_account_state(&existing_id).unwrap();
        assert_eq!(existing_account.balance(), BitcoinAmount::from_sat(5000));
        assert_eq!(existing_account.serial(), existing_serial);

        // Verify new account was created.
        let new_account = state.get_account_state(&new_id).unwrap();
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
