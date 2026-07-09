//! OL state layer that stores writes into a write batch.
//!
//! This provides an `IStateAccessor` implementation that tracks all writes
//! in a `WriteBatch`, allowing them to be applied atomically or discarded.

use std::{fmt, iter};

use strata_acct_types::{
    AccountId, AccountSerial, BitcoinAmount, L1BlockRecord, Mmr64, append_l1_block_rec_to_mmr,
};
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::*;
use strata_ol_state_types::{MAX_PENDING_ASM_LOGS, WriteBatch};

/// Helper trait for computing the state root after hypothetically applying a
/// write batch, without requiring `Clone` on the state itself.
///
/// Impls are expected to clone only what is necessary internally.
pub trait IComputeStateRootWithWrites: IStateAccessor {
    /// Computes the state root as if `batch` had been applied on top of the
    /// current state.
    fn compute_state_root_with_writes<'b>(
        &'b self,
        writes: impl Iterator<Item = &'b WriteBatch<Self::AccountState>>,
    ) -> StateResult<Buf32>
    where
        Self::AccountState: 'b;
}

/// A write-tracking state accessor that wraps a base state.
///
/// All reads check the write batch first, then fall back to the base state.
/// All writes are recorded in the write batch.
pub struct WriteTrackingState<'base, S: IStateAccessor> {
    base: &'base S,
    batch: WriteBatch<S::AccountState>,
}

impl<S: IStateAccessor> fmt::Debug for WriteTrackingState<'_, S>
where
    S: fmt::Debug,
    S::AccountState: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

    /// Creates a new write-tracking state with an empty batch.
    pub fn new_empty(base: &'base S) -> Self {
        Self {
            base,
            batch: WriteBatch::default(),
        }
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

impl<'base, S: IStateAccessor + IComputeStateRootWithWrites> IStateAccessor
    for WriteTrackingState<'base, S>
where
    S::AccountState: Clone + IAccountState + IAccountStateMut,
{
    type AccountState = S::AccountState;

    // ===== Global state methods =====

    fn cur_slot(&self) -> u64 {
        self.batch
            .global_writes()
            .cur_slot
            .unwrap_or_else(|| self.base.cur_slot())
    }

    fn limbo_funds(&self) -> BitcoinAmount {
        self.batch
            .global_writes()
            .limbo_funds_sats
            .map(BitcoinAmount::from_sat)
            .unwrap_or_else(|| self.base.limbo_funds())
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.batch
            .epochal_writes()
            .cur_epoch
            .unwrap_or_else(|| self.base.cur_epoch())
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.batch
            .epochal_writes()
            .last_l1_blkid
            .as_ref()
            .unwrap_or_else(|| self.base.last_l1_blkid())
    }

    fn last_l1_height(&self) -> L1Height {
        self.batch
            .epochal_writes()
            .last_l1_height
            .unwrap_or_else(|| self.base.last_l1_height())
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.batch
            .epochal_writes()
            .asm_recorded_epoch
            .as_ref()
            .unwrap_or_else(|| self.base.asm_recorded_epoch())
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.batch
            .epochal_writes()
            .total_ledger_balance
            .unwrap_or_else(|| self.base.total_ledger_balance())
    }

    fn l1_block_refs_mmr(&self) -> &Mmr64 {
        self.batch
            .epochal_writes()
            .l1_block_refs_mmr
            .as_ref()
            .unwrap_or_else(|| self.base.l1_block_refs_mmr())
    }

    // ===== Intraepoch state methods =====

    fn pending_asm_logs_len(&self) -> usize {
        let base_len = if self.batch.intraepoch_writes().reset {
            0
        } else {
            self.base.pending_asm_logs_len()
        };
        base_len
            + self
                .batch
                .intraepoch_writes()
                .appended_pending_asm_logs
                .len()
    }

    fn get_pending_asm_log(&self, idx: usize) -> Option<PendingAsmLog> {
        let iw = self.batch.intraepoch_writes();
        let base_len = if iw.reset {
            0
        } else {
            self.base.pending_asm_logs_len()
        };
        if idx < base_len {
            self.base.get_pending_asm_log(idx)
        } else {
            iw.appended_pending_asm_logs.get(idx - base_len).cloned()
        }
    }

    fn pending_asm_logs_full(&self) -> bool {
        self.pending_asm_logs_len() as u64 == MAX_PENDING_ASM_LOGS
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> StateResult<bool> {
        // Check write batch first
        if self.batch.ledger().contains_account(&id) {
            return Ok(true);
        }
        // Fall back to base state
        self.base.check_account_exists(id)
    }

    fn get_account_state(&self, id: AccountId) -> StateResult<Option<&Self::AccountState>> {
        // Check write batch first
        if let Some(state) = self.batch.ledger().get_account(&id) {
            return Ok(Some(state));
        }
        // Fall back to base state
        self.base.get_account_state(id)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> StateResult<Option<AccountId>> {
        // Check write batch first (for newly created accounts)
        if let Some(id) = self.batch.ledger().find_id_by_serial(serial) {
            return Ok(Some(id));
        }
        // Fall back to base state
        self.base.find_account_id_by_serial(serial)
    }

    fn next_account_serial(&self) -> AccountSerial {
        let base_serial: u32 = self.base.next_account_serial().into();
        let new_count = self.batch.ledger().new_accounts().len() as u32;
        AccountSerial::from(base_serial + new_count)
    }

    fn compute_state_root(&self) -> StateResult<Buf32> {
        self.base
            .compute_state_root_with_writes(iter::once(&self.batch))
    }
}

impl<'base, S: IStateAccessor + IComputeStateRootWithWrites> IStateAccessorMut
    for WriteTrackingState<'base, S>
where
    // FIXME(STR-3229): make this actually wrap the account state type so it
    // doesn't have to be mut on its own
    S::AccountState: IAccountStateMut,
{
    type AccountStateMut = S::AccountState; // Same type as AccountState for this layer

    fn set_cur_slot(&mut self, slot: u64) {
        self.batch.global_writes_mut().cur_slot = Some(slot);
    }

    fn add_limbo_funds_coin(&mut self, coin: Coin) -> StateResult<()> {
        let cur = self.limbo_funds();
        let amt = coin.amt();
        let Some(new) = cur.checked_add(amt) else {
            // Defuse the coin before returning: the whole STF is discarded on
            // this error, so no value is actually lost, and dropping a live coin
            // would panic in `Coin::drop`.
            coin.safely_consume_unchecked();
            return Err(StateError::LimboFundsOverflow { cur, add: amt });
        };
        self.batch.global_writes_mut().limbo_funds_sats = Some(new.to_sat());
        coin.safely_consume_unchecked();
        Ok(())
    }

    fn take_limbo_funds_coin(&mut self, amt: BitcoinAmount) -> StateResult<Coin> {
        let cur = self.limbo_funds();
        let new = cur
            .checked_sub(amt)
            .ok_or(StateError::InsufficientLimboFunds {
                need: amt,
                have: cur,
            })?;
        self.batch.global_writes_mut().limbo_funds_sats = Some(new.to_sat());
        Ok(Coin::new_unchecked(amt))
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.batch.epochal_writes_mut().cur_epoch = Some(epoch);
    }

    fn append_l1_block_rec(&mut self, height: L1Height, rec: L1BlockRecord) {
        // For append_manifest, we need to get the current MMR (from batch or
        // base), clone it, append, and store back.
        let mut mmr = self
            .batch
            .epochal_writes()
            .l1_block_refs_mmr
            .clone()
            .unwrap_or_else(|| self.base.l1_block_refs_mmr().clone());

        append_l1_block_rec_to_mmr(&mut mmr, &rec);

        let blkid = L1BlockId::from(Buf32::from(rec.block_hash()));
        let ew = self.batch.epochal_writes_mut();
        ew.l1_block_refs_mmr = Some(mmr);
        ew.last_l1_blkid = Some(blkid);
        ew.last_l1_height = Some(height);
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.batch.epochal_writes_mut().asm_recorded_epoch = Some(epoch);
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.batch.epochal_writes_mut().total_ledger_balance = Some(amt);
    }

    fn try_append_pending_asm_log(&mut self, entry: PendingAsmLog) -> StateResult<()> {
        ensure_pending_asm_log_slot_available(self.pending_asm_logs_len())?;
        self.batch
            .intraepoch_writes_mut()
            .appended_pending_asm_logs
            .push(entry);
        Ok(())
    }

    fn reset_intraepoch_state(&mut self) {
        let iw = self.batch.intraepoch_writes_mut();
        iw.reset = true;
        iw.appended_pending_asm_logs.clear();
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> StateResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        // Copy-on-write: ensure account is in batch
        if !self.batch.ledger().contains_account(&id) {
            let account = self
                .base
                .get_account_state(id)?
                .ok_or(StateError::MissingAccount(id))?
                .clone();
            self.batch.ledger_mut().update_account(id, account);
        }

        // Get mut ref from batch and run closure
        let account = self
            .batch
            .ledger_mut()
            .get_account_mut(&id)
            .expect("state: account should be in batch");
        Ok(f(account))
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData,
    ) -> StateResult<AccountSerial> {
        // Guard against duplicates so recreating over an existing account is a
        // hard error, matching the base layer. `check_account_exists` covers
        // both the pending batch and the base via its read fall-through.
        if self.check_account_exists(id)? {
            return Err(StateError::AccountExists(id));
        }
        let serial = self.next_account_serial();
        self.batch
            .ledger_mut()
            .create_account_from_data(id, new_acct_data, serial);
        Ok(serial)
    }
}

fn ensure_pending_asm_log_slot_available(current_len: usize) -> StateResult<()> {
    if current_len as u64 >= MAX_PENDING_ASM_LOGS {
        return Err(StateError::PendingAsmLogsFull);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_identifiers::L1Height;
    use strata_ol_state_types::{IStateBatchApplicable, OLAccountState};

    use super::*;
    use crate::{
        batch_diff_layer::BatchDiffState,
        common_tests::{WriteTrackingLeaf, impl_mut_layer_tests, impl_read_layer_tests},
        memory_state_layer::MemoryStateBaseLayer,
        test_utils::*,
    };

    // Shared behavior suites for a `WriteTrackingState` directly over the base.
    impl_read_layer_tests!(WriteTrackingLeaf);
    impl_mut_layer_tests!(WriteTrackingLeaf);

    // =========================================================================
    // Copy-on-write tests
    // =========================================================================

    #[test]
    fn test_write_copies_to_batch() {
        let account_id = test_account_id(1);
        let (base_layer, _) =
            setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let original_balance = base_layer
            .get_account_state(account_id)
            .unwrap()
            .unwrap()
            .balance();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        // Modify account
        tracking
            .update_account(account_id, |acct: &mut OLAccountState| {
                let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
                acct.add_balance(coin);
            })
            .unwrap();

        // Verify it's now in batch
        assert!(tracking.batch().ledger().contains_account(&account_id));

        // Verify the modified balance through tracking state
        let modified_account = tracking.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(modified_account.balance(), BitcoinAmount::from_sat(1500));

        // Verify base state is unchanged
        let base_account = base_layer.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(base_account.balance(), original_balance);
    }

    // =========================================================================
    // Account creation tests
    // =========================================================================

    #[test]
    fn test_create_account_in_batch() {
        let base_layer = create_test_base_layer();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        let account_id = test_account_id(1);
        let snark_state = test_snark_account_state(1);
        let new_acct = test_new_snark_account_data(&snark_state, BitcoinAmount::from_sat(5000));

        let serial = tracking.create_new_account(account_id, new_acct).unwrap();

        // Verify it's in the batch
        assert!(tracking.batch().ledger().contains_account(&account_id));

        // Verify we can retrieve it
        let account = tracking.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));

        // Verify base is unchanged
        assert!(!base_layer.check_account_exists(account_id).unwrap());
    }

    // =========================================================================
    // Global/epochal state tests
    // =========================================================================

    #[test]
    fn test_slot_modifications_in_batch() {
        let base_layer = create_test_base_layer();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        assert_eq!(tracking.cur_slot(), 0);

        tracking.set_cur_slot(42);

        assert_eq!(tracking.cur_slot(), 42);

        // Verify it's in the batch
        assert_eq!(tracking.batch().global_writes().cur_slot, Some(42));
    }

    #[test]
    fn test_epoch_modifications_in_batch() {
        let base_layer = create_test_base_layer();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        assert_eq!(tracking.cur_epoch(), 0);

        tracking.set_cur_epoch(5);

        assert_eq!(tracking.cur_epoch(), 5);

        // Verify it's in the batch
        assert_eq!(tracking.batch().epochal_writes().cur_epoch, Some(5));
    }

    // =========================================================================
    // State root tests
    // =========================================================================

    #[test]
    fn test_compute_state_root_with_writes() {
        let base_layer = create_test_base_layer();
        let base_root = base_layer.compute_state_root().unwrap();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        tracking.set_cur_slot(42);

        let root = tracking
            .compute_state_root()
            .expect("state root should succeed");

        // Verify it matches what we'd get by applying the batch manually.
        // (State root is currently a stub that always returns zero, so we just
        // verify the two paths are consistent rather than checking for a
        // non-trivial value.)
        let mut expected = MemoryStateBaseLayer::new(create_test_genesis_state());
        expected.apply_write_batch(tracking.into_batch()).unwrap();
        assert_eq!(root, expected.compute_state_root().unwrap());
        let _ = base_root;
    }

    // =========================================================================
    // Batch extraction tests
    // =========================================================================

    #[test]
    fn test_into_batch_returns_modifications() {
        let account_id = test_account_id(1);
        let (base_layer, _) =
            setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        // Make some modifications
        tracking.set_cur_slot(100);
        tracking
            .update_account(account_id, |acct: &mut OLAccountState| {
                let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
                acct.add_balance(coin);
            })
            .unwrap();

        // Extract the batch
        let batch = tracking.into_batch();

        // Verify modifications are in the batch
        assert_eq!(batch.global_writes().cur_slot, Some(100));
        assert!(batch.ledger().contains_account(&account_id));

        let account = batch.ledger().get_account(&account_id).unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1500));
    }

    #[test]
    fn test_batch_reference_accessible() {
        let base_layer = create_test_base_layer();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let tracking = WriteTrackingState::new_empty(&diff);

        // Should be able to access batch via reference
        let batch_ref = tracking.batch();
        assert_eq!(batch_ref.global_writes().cur_slot, None);
    }

    // =========================================================================
    // Intraepoch pending ASM log bookkeeping
    // =========================================================================

    fn pending_log(tag: u8) -> PendingAsmLog {
        let entry = strata_asm_manifest_types::AsmLogEntry::from_raw(vec![tag])
            .expect("bytes within capacity");
        PendingAsmLog::new(L1Height::from(tag as u32), entry)
    }

    fn seed_base_with_pending(count: usize) -> MemoryStateBaseLayer {
        let mut base = create_test_base_layer();
        for i in 0..count {
            base.try_append_pending_asm_log(pending_log(i as u8))
                .expect("base append");
        }
        base
    }

    #[test]
    fn test_reset_clears_prior_batch_appends() {
        let base = seed_base_with_pending(1);
        let diff = BatchDiffState::new(&base, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        tracking
            .try_append_pending_asm_log(pending_log(10))
            .expect("append before reset");
        assert_eq!(tracking.pending_asm_logs_len(), 2);

        tracking.reset_intraepoch_state();
        assert_eq!(tracking.pending_asm_logs_len(), 0);

        tracking
            .try_append_pending_asm_log(pending_log(20))
            .expect("append after reset");
        assert_eq!(tracking.pending_asm_logs_len(), 1);
        assert_eq!(
            tracking.get_pending_asm_log(0).unwrap().height(),
            L1Height::from(20u32)
        );
    }

    #[test]
    fn test_append_returns_full_at_capacity() {
        use strata_ol_state_types::MAX_PENDING_ASM_LOGS;

        ensure_pending_asm_log_slot_available(MAX_PENDING_ASM_LOGS as usize - 1)
            .expect("one slot remains");
        let overflow = ensure_pending_asm_log_slot_available(MAX_PENDING_ASM_LOGS as usize);
        assert!(matches!(overflow, Err(StateError::PendingAsmLogsFull)));
    }
}
