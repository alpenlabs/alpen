//! Toplevel state.

use bitcoin::absolute;
use strata_acct_types::{AccountId, AccountSerial, AcctError, AcctResult, BitcoinAmount};
use strata_codec::{Codec, encode_to_vec};
use strata_codec_derive::Codec;
use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, OLBlockId, hash::raw};
use strata_ledger_types::{AccountTypeState, EpochCommitment, StateAccessor};

use crate::{
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
    pub fn new_at(epoch: u64, slot: u64) -> Self {
        Self {
            epoch: EpochalState::new(
                BitcoinAmount::from(0),
                epoch as u32,
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
}

impl StateAccessor for OLState {
    type GlobalState = GlobalState;
    type L1ViewState = EpochalState;
    type AccountState = AccountState;

    fn global(&self) -> &Self::GlobalState {
        &self.global
    }

    fn global_mut(&mut self) -> &mut Self::GlobalState {
        &mut self.global
    }

    fn l1_view(&self) -> &Self::L1ViewState {
        &self.epoch
    }

    fn l1_view_mut(&mut self) -> &mut Self::L1ViewState {
        &mut self.epoch
    }

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        Ok(self.ledger.get_account_state(&id).is_some())
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        Ok(self.ledger.get_account_state(&id))
    }

    fn get_account_state_mut(
        &mut self,
        id: AccountId,
    ) -> AcctResult<Option<&mut Self::AccountState>> {
        Ok(self.ledger.get_account_state_mut(&id))
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
        let encoded = encode_to_vec(self).expect("state encoding should always succeed");
        let hash = raw(&encoded);
        Ok(hash)
    }
}

