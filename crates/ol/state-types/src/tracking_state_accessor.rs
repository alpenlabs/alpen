//! Generic tracking accessor that wraps any StateAccessor and tracks all modifications
//! during block execution, accumulating them into a WriteBatch.

use std::fmt;

use strata_acct_types::{
    AcctError, AccountId, AccountSerial, AccountTypeId, AcctResult, BitcoinAmount, Hash,
};
use strata_identifiers::{Buf32, EpochCommitment, L1Height};
use strata_ledger_types::{
    AccountTypeState, AsmManifest, Coin, IAccountState, IGlobalState, IL1ViewState,
    ISnarkAccountState, StateAccessor,
};
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::writebatch::{ExecutionAuxiliaryData, WriteBatch};

/// Tracks all state changes for WriteBatch generation using CoW overlay.
/// Generic over any StateAccessor implementation.
pub struct TrackingStateAccessor<S: StateAccessor> {
    /// Base state before execution
    base: S,

    /// Copy-on-Write overlay tracking modifications during execution
    writebatch: WriteBatch<S>,

    /// Accumulate auxiliary data for database persistence
    aux: ExecutionAuxiliaryData,
}

impl<S: StateAccessor + fmt::Debug> fmt::Debug for TrackingStateAccessor<S>
where
    S::GlobalState: fmt::Debug,
    S::L1ViewState: fmt::Debug,
    S::AccountState: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackingStateAccessor")
            .field("base", &self.base)
            .field("writebatch", &self.writebatch)
            .field("aux", &self.aux)
            .finish()
    }
}

impl<S: StateAccessor> TrackingStateAccessor<S> {
    /// Create a new state accessor from an initial state
    pub fn new(state: S) -> Self {
        Self {
            base: state,
            writebatch: WriteBatch::new(),
            aux: ExecutionAuxiliaryData::default(),
        }
    }

    /// Finalize execution and produce WriteBatch and auxiliary data
    pub fn finalize_as_writebatch(self) -> (WriteBatch<S>, ExecutionAuxiliaryData) {
        (self.writebatch, self.aux)
    }

    /// Get reference to the base state (before modifications)
    pub fn base_state(&self) -> &S {
        &self.base
    }

    /// Get reference to the writebatch overlay
    pub fn writebatch(&self) -> &WriteBatch<S> {
        &self.writebatch
    }

    /// Get mutable account, cloning from base if not in overlay
    fn get_account_mut(&mut self, acct_id: AccountId) -> AcctResult<S::AccountState>
    where
        S::AccountState: Clone,
    {
        if let Some(acct) = self.writebatch.get_account(&acct_id) {
            Ok(acct.clone())
        } else {
            self.base
                .get_account_state(acct_id)?
                .ok_or(AcctError::MissingExpectedAccount(acct_id))
                .cloned()
        }
    }

    /// Extract snark account state from account
    fn get_snark_state_mut(
        acct: &mut S::AccountState,
    ) -> AcctResult<<S::AccountState as IAccountState>::SnarkAccountState>
    where
        S::AccountState: IAccountState,
    {
        match acct.get_type_state()? {
            AccountTypeState::Snark(s) => Ok(s),
            _ => Err(AcctError::MismatchedType(acct.ty()?, AccountTypeId::Snark)),
        }
    }
}

impl<S: StateAccessor> StateAccessor for TrackingStateAccessor<S>
where
    S::GlobalState: Clone,
    S::L1ViewState: Clone,
    S::AccountState: Clone + IAccountState,
{
    type GlobalState = S::GlobalState;
    type L1ViewState = S::L1ViewState;
    type AccountState = S::AccountState;

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
        let mut acct = self.get_account_mut(acct_id)?;
        acct.add_balance(coin);
        self.writebatch.insert_account(acct_id, acct);
        Ok(())
    }

    fn take_balance(&mut self, acct_id: AccountId, amt: BitcoinAmount) -> AcctResult<Coin> {
        let mut acct = self.get_account_mut(acct_id)?;
        let coin = acct.take_balance(amt)?;
        self.writebatch.insert_account(acct_id, acct);
        Ok(coin)
    }

    fn insert_inbox_message(&mut self, acct_id: AccountId, entry: MessageEntry) -> AcctResult<()> {
        self.aux
            .account_message_additions
            .entry(acct_id)
            .or_default()
            .push(entry.clone());

        let mut acct = self.get_account_mut(acct_id)?;
        let mut snark_state = Self::get_snark_state_mut(&mut acct)?;
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
        let mut acct = self.get_account_mut(acct_id)?;
        let mut snark_state = Self::get_snark_state_mut(&mut acct)?;
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
        let serial = self.base.create_new_account(id, state)?;
        let acct = self
            .base
            .get_account_state(id)?
            .ok_or(AcctError::MissingExpectedAccount(id))?
            .clone();
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
