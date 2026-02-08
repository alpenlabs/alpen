//! Read-only state accessor that overlays WriteBatch diffs on a base state.
//!
//! This provides an `IStateAccessor` implementation that checks a stack of
//! `WriteBatch` references before falling back to a base state. All write
//! operations are unsupported since this is read-only.

use std::fmt;

use strata_acct_types::{AccountId, AccountSerial, AcctError, AcctResult, BitcoinAmount, Mmr64};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::{IStateAccessor, NewAccountData};
use strata_ol_state_types::{IStateBatchApplicable, WriteBatch};

/// A read-only state accessor that overlays a stack of WriteBatch diffs.
///
/// Reads check each batch in reverse order (last = most recent), then fall back
/// to the base state. All write operations return `AcctError::Unsupported` or
/// silently no-op (for setters that return `()`).
///
/// The batch slice can be empty, making this a read-only wrapper for the base.
/// This is useful for scenarios where you want to view state with pending
/// changes applied without modifying anything.
#[derive(Clone)]
pub struct BatchDiffState<'batches, 'base, S: IStateAccessor> {
    base: &'base S,
    batches: &'batches [WriteBatch<S::AccountState>],
}

impl<S: IStateAccessor> fmt::Debug for BatchDiffState<'_, '_, S>
where
    S: fmt::Debug,
    S::AccountState: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BatchDiffState")
            .field("base", &self.base)
            .field("batches", &self.batches)
            .finish()
    }
}

impl<'batches, 'base, S: IStateAccessor> BatchDiffState<'batches, 'base, S> {
    /// Creates a new batch diff state wrapping the given base state with a stack of batches.
    ///
    /// The batches are checked in reverse order (last = most recent) before falling
    /// back to the base state. An empty batch slice results in a pure read-only
    /// passthrough to the base.
    pub fn new(base: &'base S, batches: &'batches [WriteBatch<S::AccountState>]) -> Self {
        Self { base, batches }
    }

    /// Returns a reference to the base state.
    pub fn base(&self) -> &'base S {
        self.base
    }

    /// Returns a reference to the batch slice.
    pub fn batches(&self) -> &'batches [WriteBatch<S::AccountState>] {
        self.batches
    }
}

impl<'batches, 'base, S: IStateAccessor> IStateAccessor for BatchDiffState<'batches, 'base, S> {
    type AccountState = S::AccountState;
    type AccountStateMut = S::AccountStateMut; // Never actually used since writes fail

    // ===== Global state methods =====

    fn cur_slot(&self) -> u64 {
        self.batches
            .last()
            .map(|b| b.global().get_cur_slot())
            .unwrap_or_else(|| self.base.cur_slot())
    }

    fn set_cur_slot(&mut self, _slot: u64) {
        #[cfg(feature = "tracing")]
        tracing::error!("BatchDiffState::set_cur_slot called on read-only state");
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.batches
            .last()
            .map(|b| b.epochal().cur_epoch())
            .unwrap_or_else(|| self.base.cur_epoch())
    }

    fn set_cur_epoch(&mut self, _epoch: u32) {
        #[cfg(feature = "tracing")]
        tracing::error!("BatchDiffState::set_cur_epoch called on read-only state");
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.batches
            .last()
            .map(|b| b.epochal().last_l1_blkid())
            .unwrap_or_else(|| self.base.last_l1_blkid())
    }

    fn last_l1_height(&self) -> L1Height {
        self.batches
            .last()
            .map(|b| b.epochal().last_l1_height())
            .unwrap_or_else(|| self.base.last_l1_height())
    }

    fn append_manifest(&mut self, _height: L1Height, _mf: AsmManifest) {
        #[cfg(feature = "tracing")]
        tracing::error!("BatchDiffState::append_manifest called on read-only state");
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.batches
            .last()
            .map(|b| b.epochal().asm_recorded_epoch())
            .unwrap_or_else(|| self.base.asm_recorded_epoch())
    }

    fn set_asm_recorded_epoch(&mut self, _epoch: EpochCommitment) {
        #[cfg(feature = "tracing")]
        tracing::error!("BatchDiffState::set_asm_recorded_epoch called on read-only state");
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.batches
            .last()
            .map(|b| b.epochal().total_ledger_balance())
            .unwrap_or_else(|| self.base.total_ledger_balance())
    }

    fn set_total_ledger_balance(&mut self, _amt: BitcoinAmount) {
        #[cfg(feature = "tracing")]
        tracing::error!("BatchDiffState::set_total_ledger_balance called on read-only state");
    }

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        self.batches
            .last()
            .map(|b| b.epochal().asm_manifests_mmr())
            .unwrap_or_else(|| self.base.asm_manifests_mmr())
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        // Check batches in reverse order (last = most recent)
        for batch in self.batches.iter().rev() {
            if batch.ledger().contains_account(&id) {
                return Ok(true);
            }
        }
        // Fall back to base state
        self.base.check_account_exists(id)
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        // Check batches in reverse order (last = most recent)
        for batch in self.batches.iter().rev() {
            if let Some(state) = batch.ledger().get_account(&id) {
                return Ok(Some(state));
            }
        }
        // Fall back to base state
        self.base.get_account_state(id)
    }

    fn update_account<R, F>(&mut self, _id: AccountId, _f: F) -> AcctResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        Err(AcctError::Unsupported)
    }

    fn create_new_account(
        &mut self,
        _id: AccountId,
        _new_acct_data: NewAccountData<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        Err(AcctError::Unsupported)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        // Check batches in reverse order (last = most recent)
        for batch in self.batches.iter().rev() {
            if let Some(id) = batch.ledger().find_id_by_serial(serial) {
                return Ok(Some(id));
            }
        }
        // Fall back to base state
        self.base.find_account_id_by_serial(serial)
    }

    fn next_account_serial(&self) -> AccountSerial {
        let total_new_accounts: u32 = self
            .batches
            .iter()
            .map(|b| b.ledger().new_accounts().len() as u32)
            .sum();
        let base_serial: u32 = self.base.next_account_serial().into();
        AccountSerial::from(base_serial + total_new_accounts)
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        Err(AcctError::Unsupported)
    }
}

impl<'batches, 'base, S: IStateAccessor> IStateBatchApplicable
    for BatchDiffState<'batches, 'base, S>
{
    fn apply_write_batch(&mut self, _batch: WriteBatch<Self::AccountState>) -> AcctResult<()> {
        Err(AcctError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AcctError, BitcoinAmount, SYSTEM_RESERVED_ACCTS};
    use strata_identifiers::{AccountSerial, Buf32, Epoch, L1BlockCommitment, L1BlockId, Slot};
    use strata_ledger_types::{AccountTypeState, IAccountState, IStateAccessor, NewAccountData};
    use strata_ol_params::OLParams;
    use strata_ol_state_types::OLState;

    use super::*;
    use crate::test_utils::*;

    fn new_ol_state_at(epoch: Epoch, slot: Slot) -> OLState {
        let mut params = OLParams::new_empty(L1BlockCommitment::default());
        params.header.slot = slot;
        params.header.epoch = epoch;
        OLState::from_genesis_params(&params).expect("failed to create OLState from genesis params")
    }

    // =========================================================================
    // Empty batch tests (pure passthrough)
    // =========================================================================

    #[test]
    fn test_read_from_base_when_empty_batches() {
        let account_id = test_account_id(1);
        let (base_state, serial) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

        let batches: Vec<WriteBatch<_>> = vec![];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        // Should read from base
        let account = diff_state.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));
    }

    #[test]
    fn test_global_state_from_base_when_empty() {
        let base_state = new_ol_state_at(5, 100);
        let batches: Vec<WriteBatch<_>> = vec![];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        assert_eq!(diff_state.cur_slot(), 100);
        assert_eq!(diff_state.cur_epoch(), 5);
    }

    #[test]
    fn test_check_account_exists_in_base_only() {
        let account_id = test_account_id(1);
        let nonexistent_id = test_account_id(99);
        let (base_state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

        let batches: Vec<WriteBatch<_>> = vec![];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        assert!(diff_state.check_account_exists(account_id).unwrap());
        assert!(!diff_state.check_account_exists(nonexistent_id).unwrap());
    }

    // =========================================================================
    // Single batch tests
    // =========================================================================

    #[test]
    fn test_read_from_single_batch() {
        let account_id = test_account_id(1);
        let base_state = create_test_genesis_state();

        // Create a batch with an account
        let mut batch = WriteBatch::new_from_state(&base_state);
        let snark_state = test_snark_account_state(1);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(5000),
            AccountTypeState::Snark(snark_state),
        );
        let serial = base_state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        // Should read from batch
        let account = diff_state.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));
    }

    #[test]
    fn test_check_account_exists_in_batch() {
        let account_id = test_account_id(1);
        let base_state = create_test_genesis_state();

        let mut batch = WriteBatch::new_from_state(&base_state);
        let snark_state = test_snark_account_state(1);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(5000),
            AccountTypeState::Snark(snark_state),
        );
        let serial = base_state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        assert!(diff_state.check_account_exists(account_id).unwrap());
    }

    #[test]
    fn test_global_state_from_top_batch() {
        let base_state = new_ol_state_at(5, 100);

        let mut batch = WriteBatch::new_from_state(&base_state);
        batch.global_mut().set_cur_slot(200);
        batch.epochal_mut().set_cur_epoch(10);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        assert_eq!(diff_state.cur_slot(), 200);
        assert_eq!(diff_state.cur_epoch(), 10);
    }

    // =========================================================================
    // Batch stack tests (multiple batches)
    // =========================================================================

    #[test]
    fn test_read_from_batch_stack_last_shadows() {
        let account_id = test_account_id(1);
        let base_state = create_test_genesis_state();

        // First batch: account with 1000 sats
        let mut batch1 = WriteBatch::new_from_state(&base_state);
        let snark_state1 = test_snark_account_state(1);
        let new_acct1 = NewAccountData::new(
            BitcoinAmount::from_sat(1000),
            AccountTypeState::Snark(snark_state1),
        );
        let serial1 = base_state.next_account_serial();
        batch1
            .ledger_mut()
            .create_account_from_data(account_id, new_acct1, serial1);

        // Second batch (more recent): same account with 5000 sats
        // This batch shadows the first, so uses a different serial
        let mut batch2 = WriteBatch::new_from_state(&base_state);
        let snark_state2 = test_snark_account_state(2);
        let new_acct2 = NewAccountData::new(
            BitcoinAmount::from_sat(5000),
            AccountTypeState::Snark(snark_state2),
        );
        let serial2 = AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1);
        batch2
            .ledger_mut()
            .create_account_from_data(account_id, new_acct2, serial2);

        // Last batch should shadow first
        let batches = vec![batch1, batch2];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        let account = diff_state.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));
    }

    #[test]
    fn test_read_falls_through_to_earlier_batch() {
        let account_id_1 = test_account_id(1);
        let account_id_2 = test_account_id(2);
        let base_state = create_test_genesis_state();

        // First batch: account 1
        let mut batch1 = WriteBatch::new_from_state(&base_state);
        let snark_state1 = test_snark_account_state(1);
        let new_acct1 = NewAccountData::new(
            BitcoinAmount::from_sat(1000),
            AccountTypeState::Snark(snark_state1),
        );
        let serial1 = base_state.next_account_serial();
        batch1
            .ledger_mut()
            .create_account_from_data(account_id_1, new_acct1, serial1);

        // Second batch: account 2 only
        let mut batch2 = WriteBatch::new_from_state(&base_state);
        let snark_state2 = test_snark_account_state(2);
        let new_acct2 = NewAccountData::new(
            BitcoinAmount::from_sat(2000),
            AccountTypeState::Snark(snark_state2),
        );
        let serial2 = AccountSerial::from(SYSTEM_RESERVED_ACCTS + 1);
        batch2
            .ledger_mut()
            .create_account_from_data(account_id_2, new_acct2, serial2);

        let batches = vec![batch1, batch2];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        // Account 1 should be found in batch1 (falls through from batch2)
        let account1 = diff_state.get_account_state(account_id_1).unwrap().unwrap();
        assert_eq!(account1.balance(), BitcoinAmount::from_sat(1000));

        // Account 2 should be found in batch2
        let account2 = diff_state.get_account_state(account_id_2).unwrap().unwrap();
        assert_eq!(account2.balance(), BitcoinAmount::from_sat(2000));
    }

    #[test]
    fn test_read_falls_through_to_base() {
        let account_id_base = test_account_id(1);
        let account_id_batch = test_account_id(2);
        let (base_state, _) =
            setup_state_with_snark_account(account_id_base, 1, BitcoinAmount::from_sat(1000));

        let mut batch = WriteBatch::new_from_state(&base_state);
        let snark_state = test_snark_account_state(2);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(2000),
            AccountTypeState::Snark(snark_state),
        );
        let serial = base_state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id_batch, new_acct, serial);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        // Account in base should be found
        let base_account = diff_state
            .get_account_state(account_id_base)
            .unwrap()
            .unwrap();
        assert_eq!(base_account.balance(), BitcoinAmount::from_sat(1000));

        // Account in batch should also be found
        let batch_account = diff_state
            .get_account_state(account_id_batch)
            .unwrap()
            .unwrap();
        assert_eq!(batch_account.balance(), BitcoinAmount::from_sat(2000));
    }

    #[test]
    fn test_find_serial_in_batch_stack() {
        let account_id = test_account_id(1);
        let base_state = create_test_genesis_state();

        let mut batch = WriteBatch::new_from_state(&base_state);
        let snark_state = test_snark_account_state(1);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(1000),
            AccountTypeState::Snark(snark_state),
        );
        let serial = base_state.next_account_serial();
        batch
            .ledger_mut()
            .create_account_from_data(account_id, new_acct, serial);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        let found_id = diff_state.find_account_id_by_serial(serial).unwrap();
        assert_eq!(found_id, Some(account_id));
    }

    // =========================================================================
    // Write operation tests (all should fail/no-op)
    // =========================================================================

    #[test]
    fn test_update_account_returns_unsupported() {
        let account_id = test_account_id(1);
        let (base_state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

        let batches: Vec<WriteBatch<_>> = vec![];
        let mut diff_state = BatchDiffState::new(&base_state, &batches);

        let result = diff_state.update_account(account_id, |_acct| {});
        assert!(matches!(result, Err(AcctError::Unsupported)));
    }

    #[test]
    fn test_create_account_returns_unsupported() {
        let base_state = create_test_genesis_state();
        let batches: Vec<WriteBatch<_>> = vec![];
        let mut diff_state = BatchDiffState::new(&base_state, &batches);

        let account_id = test_account_id(1);
        let snark_state = test_snark_account_state(1);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(1000),
            AccountTypeState::Snark(snark_state),
        );

        let result = diff_state.create_new_account(account_id, new_acct);
        assert!(matches!(result, Err(AcctError::Unsupported)));
    }

    #[test]
    fn test_compute_state_root_returns_unsupported() {
        let base_state = create_test_genesis_state();
        let batches: Vec<WriteBatch<_>> = vec![];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        let result = diff_state.compute_state_root();
        assert!(matches!(result, Err(AcctError::Unsupported)));
    }

    // =========================================================================
    // Epochal state tests
    // =========================================================================

    #[test]
    fn test_epochal_state_from_top_batch() {
        let base_state = create_test_genesis_state();

        let mut batch = WriteBatch::new_from_state(&base_state);
        batch
            .epochal_mut()
            .set_total_ledger_balance(BitcoinAmount::from_sat(1_000_000));

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        assert_eq!(
            diff_state.total_ledger_balance(),
            BitcoinAmount::from_sat(1_000_000)
        );
    }

    #[test]
    fn test_last_l1_blkid_from_batch() {
        let base_state = create_test_genesis_state();
        let batch = WriteBatch::new_from_state(&base_state);

        let batches = vec![batch];
        let diff_state = BatchDiffState::new(&base_state, &batches);

        // Should return the L1 block ID from the batch's epochal state
        let blkid = diff_state.last_l1_blkid();
        assert_eq!(*blkid, L1BlockId::from(Buf32::zero()));
    }
}
