//! Base state layer for [`OLState`].

use std::collections::BTreeMap;

use strata_acct_types::{AccountId, AccountSerial, AcctError, AcctResult, BitcoinAmount, Mmr64};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::{IStateAccessor, NewAccountData};
use strata_ol_state_types::{IStateBatchApplicable, OLAccountState, OLState, WriteBatch};

use crate::write_tracking_layer::IComputeStateRootWithWrites;

/// Base layer wrapping [`OLState`].
#[derive(Clone, Debug)]
pub struct MemoryStateBaseLayer {
    state: OLState,
    serials: BTreeMap<AccountSerial, AccountId>,
}

impl MemoryStateBaseLayer {
    /// Constructs a new instance.  Indexes the serials in the process.
    ///
    /// # Panics
    ///
    /// If the state's accounts have duplicated serials.
    pub fn new(state: OLState) -> Self {
        let serials: BTreeMap<_, _> = state
            .ledger
            .accounts
            .iter()
            .map(|a| (a.state.serial, a.id))
            .collect();

        assert_eq!(
            serials.len(),
            state.ledger.accounts.len(),
            "ol/state-support: state has duplicated serials"
        );

        Self { state, serials }
    }

    pub fn state(&self) -> &OLState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut OLState {
        &mut self.state
    }

    pub fn into_inner(self) -> OLState {
        self.state
    }
}

impl IStateAccessor for MemoryStateBaseLayer {
    type AccountState = OLAccountState;
    type AccountStateMut = OLAccountState;

    // ===== Global state methods =====

    fn cur_slot(&self) -> u64 {
        self.state.global.get_cur_slot()
    }

    fn set_cur_slot(&mut self, slot: u64) {
        self.state.global.set_cur_slot(slot);
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.state.epoch.cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.state.epoch.set_cur_epoch(epoch);
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.state.epoch.last_l1_blkid()
    }

    fn last_l1_height(&self) -> L1Height {
        self.state.epoch.last_l1_height()
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        self.state.epoch.append_manifest(height, mf);
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.state.epoch.asm_recorded_epoch()
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.state.epoch.set_asm_recorded_epoch(epoch);
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.state.epoch.total_ledger_balance()
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.state.epoch.set_total_ledger_balance(amt);
    }

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        self.state.epoch.asm_manifests_mmr()
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        Ok(self.state.ledger.get_account_state(&id).is_some())
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        Ok(self.state.ledger.get_account_state(&id))
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> AcctResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        let acct = self
            .state
            .ledger
            .get_account_state_mut(&id)
            .ok_or(AcctError::MissingExpectedAccount(id))?;
        Ok(f(acct))
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData,
    ) -> AcctResult<AccountSerial> {
        let serial = self.state.global.get_next_avail_serial();
        let mut batch = WriteBatch::<OLAccountState>::default();
        batch
            .ledger_mut()
            .create_account_from_data(id, new_acct_data, serial);
        self.state.apply_write_batch(batch)?;
        self.serials.insert(serial, id);
        Ok(serial)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        Ok(self.serials.get(&serial).copied())
    }

    fn next_account_serial(&self) -> AccountSerial {
        self.state.global.get_next_avail_serial()
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        // TODO: use a proper state root computation
        Ok(Buf32::zero())
    }
}

impl IStateBatchApplicable for MemoryStateBaseLayer {
    fn apply_write_batch(&mut self, batch: WriteBatch<Self::AccountState>) -> AcctResult<()> {
        self.state.apply_write_batch(batch)
    }
}

impl IComputeStateRootWithWrites for MemoryStateBaseLayer {
    fn compute_state_root_with_writes(
        &self,
        batch: WriteBatch<OLAccountState>,
    ) -> AcctResult<Buf32> {
        let mut state = self.state.clone();
        state.apply_write_batch(batch)?;
        // TODO: use a proper state root computation
        let _ = state;
        Ok(Buf32::zero())
    }
}
