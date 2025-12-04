//! Toplevel state.

use bitcoin::absolute;
use strata_acct_types::{AccountId, AccountSerial, AcctError, AcctResult, BitcoinAmount};
use strata_asm_manifest_types::AsmManifest;
use strata_codec::{Codec, encode_to_vec};
use strata_identifiers::{
    Buf32, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, L1Height, OLBlockId, Slot,
    hash::raw,
};
use strata_ledger_types::{AccountTypeState, IStateAccessor};

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
}

impl IStateAccessor for OLState {
    type AccountState = AccountState;

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

    // ===== Account methods =====

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
        // TODO change this to use SSZ
        let encoded = encode_to_vec(self).expect("ol/state: encode");
        let hash = raw(&encoded);
        Ok(hash)
    }
}
