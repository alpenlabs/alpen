//! This accessor wraps OLState and tracks all modifications during block execution,
//! accumulating them into a WriteBatch that can be persisted to the database.

use std::collections::{HashMap, HashSet};

use strata_acct_types::{AccountId, AccountSerial, AcctResult, BitcoinAmount, Hash, Mmr64};
use strata_identifiers::{Buf32, EpochCommitment, L1Height};
use strata_ledger_types::{
    AccountTypeState, AsmManifest, Coin, IGlobalState, IL1ViewState, StateAccessor,
};
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::{
    AccountState, EpochalState, GlobalState, OLState,
    writebatch::{ExecutionAuxiliaryData, L1ViewWrites, WriteBatch},
};

/// Tracks all state changes for WriteBatch generation
#[derive(Debug)]
pub struct TrackingStateAccessor {
    /// Original state before execution (for debugging/comparison)
    original: OLState,

    /// Current state being modified during execution
    current: OLState,

    /// Track which accounts were modified (excluding newly created)
    modified_accounts: HashSet<AccountId>,

    /// Track which accounts were newly created
    created_accounts: HashSet<AccountId>,

    /// Accumulate auxiliary data for database persistence
    aux: ExecutionAuxiliaryData,
}

impl TrackingStateAccessor {
    /// Create a new state accessor from an initial state
    pub fn new(state: OLState) -> Self {
        Self {
            original: state.clone(),
            current: state,
            modified_accounts: HashSet::new(),
            created_accounts: HashSet::new(),
            aux: ExecutionAuxiliaryData::default(),
        }
    }

    /// Finalize execution and produce WriteBatch and auxiliary data
    pub fn finalize_as_writebatch(self) -> (WriteBatch, ExecutionAuxiliaryData) {
        // Extract modified accounts (excluding newly created ones)
        let mut modified_accounts = HashMap::new();
        for id in &self.modified_accounts {
            if !self.created_accounts.contains(id)
                && let Ok(Some(acct)) = self.current.get_account_state(*id)
            {
                modified_accounts.insert(*id, acct.clone());
            }
        }

        // Extract new accounts
        let mut new_accounts = Vec::new();
        for id in &self.created_accounts {
            if let Ok(Some(acct)) = self.current.get_account_state(*id) {
                let serial = acct.serial();
                new_accounts.push((*id, serial, acct.clone()));
            }
        }

        let asm_mmr = Mmr64::new(64); // FIXME: why need to pass 64??

        let write_batch = WriteBatch {
            new_slot: self.current.global().cur_slot(),
            l1_view_writes: L1ViewWrites {
                cur_epoch: self.current.l1_view().cur_epoch(),
                added_manifests: self.aux.asm_manifests.clone(),
                asm_manifest_mmr: asm_mmr,
                asm_recorded_epoch: *self.current.l1_view().asm_recorded_epoch(),
                total_ledger_balance: self.current.l1_view().total_ledger_balance(),
            },
            new_accounts,
            modified_accounts,
            // TODO: fix this, this should be ledger's root, not state root
            ledger_state_root: self
                .current
                .compute_state_root()
                .expect("failed to compute state root"),
        };

        (write_batch, self.aux)
    }

    /// Get reference to the original state (before modifications)
    pub fn original_state(&self) -> &OLState {
        &self.original
    }

    /// Get reference to the current state (with modifications)
    pub fn current_state(&self) -> &OLState {
        &self.current
    }
}

impl StateAccessor for TrackingStateAccessor {
    type GlobalState = GlobalState;
    type L1ViewState = EpochalState;
    type AccountState = AccountState;

    fn global(&self) -> &Self::GlobalState {
        self.current.global()
    }

    fn set_cur_slot(&mut self, slot: u64) {
        self.current.set_cur_slot(slot);
    }

    fn l1_view(&self) -> &Self::L1ViewState {
        self.current.l1_view()
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.current.set_cur_epoch(epoch);
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        // Accumulate for auxiliary data
        self.aux.asm_manifests.push(mf.clone());

        // Apply to current state
        self.current.append_manifest(height, mf);
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.current.set_asm_recorded_epoch(epoch);
    }

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        self.current.check_account_exists(id)
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        self.current.get_account_state(id)
    }

    fn add_balance(&mut self, acct_id: AccountId, coin: Coin) -> AcctResult<()> {
        self.current.add_balance(acct_id, coin)?;
        // TODO: might need to track added balance
        self.modified_accounts.insert(acct_id);
        Ok(())
    }

    fn take_balance(&mut self, acct_id: AccountId, amt: BitcoinAmount) -> AcctResult<Coin> {
        let coin = self.current.take_balance(acct_id, amt)?;
        // TODO: might need to track taken balance
        self.modified_accounts.insert(acct_id);
        Ok(coin)
    }

    fn insert_inbox_message(&mut self, acct_id: AccountId, entry: MessageEntry) -> AcctResult<()> {
        // Accumulate for auxiliary data
        self.aux
            .account_message_additions
            .entry(acct_id)
            .or_default()
            .push(entry.clone());

        // Apply to current state
        self.current.insert_inbox_message(acct_id, entry)?;
        self.modified_accounts.insert(acct_id);
        Ok(())
    }

    fn set_proof_state_directly(
        &mut self,
        acct_id: AccountId,
        state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
    ) -> AcctResult<()> {
        self.current
            .set_proof_state_directly(acct_id, state, next_read_idx, seqno)?;
        self.modified_accounts.insert(acct_id);
        Ok(())
    }

    fn update_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()> {
        self.current.update_account_state(id, state)?;
        self.modified_accounts.insert(id);
        Ok(())
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        let serial = self.current.create_new_account(id, state)?;
        self.created_accounts.insert(id);
        Ok(serial)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        self.current.find_account_id_by_serial(serial)
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        self.current.compute_state_root()
    }
}

// TODO: comprehensive tests
