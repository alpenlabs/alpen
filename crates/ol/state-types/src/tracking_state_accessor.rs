//! This accessor wraps OLState and tracks all modifications during block execution,
//! accumulating them into a WriteBatch that can be persisted to the database.

use strata_acct_types::{AccountId, AccountSerial, AcctResult, BitcoinAmount, Hash};
use strata_identifiers::{Buf32, EpochCommitment, L1Height};
use strata_ledger_types::{
    AccountTypeState, AsmManifest, Coin, IAccountState, IGlobalState, IL1ViewState,
    ISnarkAccountState, StateAccessor,
};
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::{
    AccountState, EpochalState, GlobalState, OLState,
    writebatch::{ExecutionAuxiliaryData, WriteBatch},
};

/// Tracks all state changes for WriteBatch generation using CoW overlay
#[derive(Debug)]
pub struct TrackingStateAccessor {
    /// Base state before execution
    base: OLState,

    /// Copy-on-Write overlay tracking modifications during execution
    writebatch: WriteBatch<OLState>,

    /// Accumulate auxiliary data for database persistence
    aux: ExecutionAuxiliaryData,
}

impl TrackingStateAccessor {
    /// Create a new state accessor from an initial state
    pub fn new(state: OLState) -> Self {
        Self {
            base: state,
            writebatch: WriteBatch::new(),
            aux: ExecutionAuxiliaryData::default(),
        }
    }

    /// Finalize execution and produce WriteBatch and auxiliary data
    pub fn finalize_as_writebatch(self) -> (WriteBatch<OLState>, ExecutionAuxiliaryData) {
        (self.writebatch, self.aux)
    }

    /// Get reference to the base state (before modifications)
    pub fn base_state(&self) -> &OLState {
        &self.base
    }

    /// Get reference to the writebatch overlay
    pub fn writebatch(&self) -> &WriteBatch<OLState> {
        &self.writebatch
    }
}

impl StateAccessor for TrackingStateAccessor {
    type GlobalState = GlobalState;
    type L1ViewState = EpochalState;
    type AccountState = AccountState;

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
        // Get or clone the account into overlay
        let acct = if let Some(acct) = self.writebatch.get_account(&acct_id) {
            acct.clone()
        } else {
            self.base
                .get_account_state(acct_id)?
                .ok_or(strata_acct_types::AcctError::MissingExpectedAccount(acct_id))?
                .clone()
        };

        let mut acct = acct;
        acct.add_balance(coin);
        self.writebatch.insert_account(acct_id, acct);
        Ok(())
    }

    fn take_balance(&mut self, acct_id: AccountId, amt: BitcoinAmount) -> AcctResult<Coin> {
        // Get or clone the account into overlay
        let acct = if let Some(acct) = self.writebatch.get_account(&acct_id) {
            acct.clone()
        } else {
            self.base
                .get_account_state(acct_id)?
                .ok_or(strata_acct_types::AcctError::MissingExpectedAccount(acct_id))?
                .clone()
        };

        let mut acct = acct;
        let coin = acct.take_balance(amt)?;
        self.writebatch.insert_account(acct_id, acct);
        Ok(coin)
    }

    fn insert_inbox_message(&mut self, acct_id: AccountId, entry: MessageEntry) -> AcctResult<()> {
        // Accumulate for auxiliary data
        self.aux
            .account_message_additions
            .entry(acct_id)
            .or_default()
            .push(entry.clone());

        // Get or clone the account into overlay
        let mut acct = if let Some(acct) = self.writebatch.get_account(&acct_id) {
            acct.clone()
        } else {
            self.base
                .get_account_state(acct_id)?
                .ok_or(strata_acct_types::AcctError::MissingExpectedAccount(acct_id))?
                .clone()
        };

        // Get snark state, modify it, and set it back
        let mut snark_state = match acct.get_type_state()? {
            AccountTypeState::Snark(s) => s,
            _ => return Err(strata_acct_types::AcctError::MismatchedType(
                acct.ty()?,
                strata_acct_types::AccountTypeId::Snark,
            )),
        };

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
        // Get or clone the account into overlay
        let mut acct = if let Some(acct) = self.writebatch.get_account(&acct_id) {
            acct.clone()
        } else {
            self.base
                .get_account_state(acct_id)?
                .ok_or(strata_acct_types::AcctError::MissingExpectedAccount(acct_id))?
                .clone()
        };

        // Get snark state, modify it, and set it back
        let mut snark_state = match acct.get_type_state()? {
            AccountTypeState::Snark(s) => s,
            _ => return Err(strata_acct_types::AcctError::MismatchedType(
                acct.ty()?,
                strata_acct_types::AccountTypeId::Snark,
            )),
        };

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
        // Create account in base state to get serial
        let serial = self.base.create_new_account(id, state)?;

        // Get the newly created account and put it in overlay
        let acct = self.base.get_account_state(id)?.ok_or(
            strata_acct_types::AcctError::MissingExpectedAccount(id)
        )?.clone();

        self.writebatch.insert_account(id, acct);
        Ok(serial)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        self.base.find_account_id_by_serial(serial)
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        // TODO: This needs to compute state root incorporating overlay changes
        // For now, delegate to base (incorrect but will compile)
        self.base.compute_state_root()
    }
}

// TODO: comprehensive tests
