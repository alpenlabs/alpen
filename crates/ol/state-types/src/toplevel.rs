//! Toplevel state.

use bitcoin::absolute;
use strata_acct_types::{
    AccountId, AccountSerial, AccountTypeId, AcctError, AcctResult, BitcoinAmount, Hash,
};
use strata_codec::{Codec, encode_to_vec};
use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, L1Height, OLBlockId, hash::raw};
use strata_ledger_types::{
    AccountTypeState, AsmManifest, Coin, EpochCommitment, IAccountState, IGlobalState,
    IL1ViewState, ISnarkAccountState, StateAccessor,
};
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::{
    NativeSnarkAccountState,
    account::{AccountState, NativeAccountTypeState},
    epochal::EpochalState,
    global::GlobalState,
    ledger::TsnlLedgerAccountsTable,
};

#[derive(Clone, Debug, Codec)]
pub struct OLState {
    epoch: EpochalState,
    global: GlobalState,
    ledger: TsnlLedgerAccountsTable,
}

impl OLState {
    /// Create a new genesis state for testing.
    pub fn new_genesis() -> Self {
        Self {
            epoch: EpochalState::new(
                BitcoinAmount::from(0),
                0,
                L1BlockCommitment::new(
                    absolute::Height::from_consensus(0).unwrap(),
                    L1BlockId::from(Buf32::zero()),
                ),
                EpochCommitment::new(0, 0, OLBlockId::from(Buf32::zero())),
            ),
            global: GlobalState::new(0),
            ledger: TsnlLedgerAccountsTable::new_empty(),
        }
    }

    /// Create a state with specified epoch and slot for testing.
    pub fn new_at(epoch: Epoch, slot: Slot) -> Self {
        Self {
            epoch: EpochalState::new(
                BitcoinAmount::from(0),
                epoch,
                L1BlockCommitment::new(
                    absolute::Height::from_consensus(0).unwrap(),
                    L1BlockId::from(Buf32::zero()),
                ),
                EpochCommitment::new(epoch, slot, OLBlockId::from(Buf32::zero())),
            ),
            global: GlobalState::new(slot),
            ledger: TsnlLedgerAccountsTable::new_empty(),
        }
    }

    fn get_acct_state_mut(&mut self, acct_id: &AccountId) -> AcctResult<&mut AccountState> {
        self.ledger
            .get_account_state_mut(acct_id)
            .ok_or(AcctError::MissingExpectedAccount(*acct_id))
    }

    fn get_acct_state(&self, acct_id: &AccountId) -> AcctResult<&AccountState> {
        self.ledger
            .get_account_state(acct_id)
            .ok_or(AcctError::MissingExpectedAccount(*acct_id))
    }

    fn get_snark_acct(&self, acct_id: &AccountId) -> AcctResult<NativeSnarkAccountState> {
        let acct_state = self.get_acct_state(acct_id)?;
        match acct_state.get_type_state()? {
            AccountTypeState::Snark(s) => Ok(s),
            _ => Err(AcctError::MismatchedType(
                AccountTypeId::Empty,
                AccountTypeId::Snark,
            )),
        }
    }

    fn set_snark_acct(
        &mut self,
        acct_id: &AccountId,
        s: NativeSnarkAccountState,
    ) -> AcctResult<()> {
        let acct_state = self.get_acct_state_mut(acct_id)?;
        match acct_state.get_type_state()? {
            AccountTypeState::Snark(_) => acct_state.set_type_state(AccountTypeState::Snark(s)),
            _ => Err(AcctError::MismatchedType(
                AccountTypeId::Empty,
                AccountTypeId::Snark,
            )),
        }
    }
}

impl StateAccessor for OLState {
    type GlobalState = GlobalState;
    type L1ViewState = EpochalState;
    type AccountState = AccountState;

    fn global(&self) -> &Self::GlobalState {
        &self.global
    }

    fn set_cur_slot(&mut self, slot: u64) {
        self.global.set_cur_slot(slot);
    }

    fn l1_view(&self) -> &Self::L1ViewState {
        &self.epoch
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.epoch.set_cur_epoch(epoch);
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.epoch.set_asm_recorded_epoch(epoch);
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        self.epoch.append_manifest(height, mf);
    }

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        Ok(self.ledger.get_account_state(&id).is_some())
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        Ok(self.ledger.get_account_state(&id))
    }

    fn add_balance(&mut self, acct_id: AccountId, coin: Coin) -> AcctResult<()> {
        let acct_state = self.get_acct_state_mut(&acct_id)?;
        acct_state.add_balance(coin);
        Ok(())
    }

    fn take_balance(&mut self, acct_id: AccountId, amt: BitcoinAmount) -> AcctResult<Coin> {
        let acct_state = self.get_acct_state_mut(&acct_id)?;
        acct_state.take_balance(amt)
    }

    fn insert_inbox_message(&mut self, acct_id: AccountId, entry: MessageEntry) -> AcctResult<()> {
        let mut snark_acct = self.get_snark_acct(&acct_id)?;
        snark_acct.insert_inbox_message(entry)?;
        self.set_snark_acct(&acct_id, snark_acct)
    }

    fn set_proof_state_directly(
        &mut self,
        acct_id: AccountId,
        state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
    ) -> AcctResult<()> {
        let mut snark_state = self.get_snark_acct(&acct_id)?;
        snark_state.set_proof_state_directly(state, next_read_idx, seqno);
        self.set_snark_acct(&acct_id, snark_state)
    }

    fn update_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()> {
        let acct = self
            .ledger
            .get_account_state_mut(&id)
            .ok_or(AcctError::MissingExpectedAccount(id))?;
        *acct = state;
        Ok(())
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        let serial = self.ledger.next_avail_serial();
        let state = NativeAccountTypeState::from_generic(state);
        let account = AccountState::new(serial, BitcoinAmount::from(0), state);
        self.ledger.create_account(id, account)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        Ok(self.ledger.get_serial_acct_id(serial).copied())
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        // Compute the state root by hashing the Codec encoding of the state
        // For now, we'll panic on encoding errors as they shouldn't happen in practice
        // TODO change this to use SSZ
        let encoded = encode_to_vec(self).expect("ol/state: encode");
        let hash = raw(&encoded);
        Ok(hash)
    }
}

impl OLState {
    /// Apply a WriteBatch to this state
    ///
    /// This takes the changes accumulated during block execution and applies them
    /// to the current state. Used when accepting blocks into the canonical chain.
    pub fn apply_write_batch(&mut self, batch: crate::WriteBatch) -> AcctResult<()> {
        // Apply global state changes
        self.global.set_cur_slot(batch.new_slot);

        // Apply L1 view changes
        self.epoch.set_cur_epoch(batch.l1_view_writes.cur_epoch);
        self.epoch
            .set_asm_recorded_epoch(batch.l1_view_writes.asm_recorded_epoch);
        self.epoch
            .set_total_ledger_balance(batch.l1_view_writes.total_ledger_balance);

        // Note: manifests were already applied during execution via append_manifest()
        // They're included in WriteBatch for DB persistence, not for state application

        // Apply new accounts
        for (id, _serial, acct_state) in batch.new_accounts {
            // Accounts should have been created during execution
            // This just ensures they're in the final state
            self.update_account_state(id, acct_state)?;
        }

        // Apply modified accounts
        for (id, acct_state) in batch.modified_accounts {
            self.update_account_state(id, acct_state)?;
        }

        Ok(())
    }
}
