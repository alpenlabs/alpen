//! OL state layer that stores writes into a write batch.
//!
//! This provides an `IStateAccessor` implementation that tracks all writes
//! in a `WriteBatch`, allowing them to be applied atomically or discarded.

use strata_acct_types::{AccountId, AccountSerial, AcctError, AcctResult, BitcoinAmount};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::{IAccountStateConstructible, IAccountStateMut, IStateAccessor, NewAccountData};

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
    type AccountStateMut = S::AccountState;  // Same type as AccountState for this layer

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
