//! OL state layer that stores writes into a write batch.
//!
//! This provides an `IStateAccessor` implementation that tracks all writes
//! in a `WriteBatch`, allowing them to be applied atomically or discarded.

use std::{fmt, iter};

use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount, Mmr64};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::*;
use strata_ol_state_types::WriteBatch;

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

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        self.batch
            .epochal_writes()
            .asm_manifests_mmr
            .as_ref()
            .unwrap_or_else(|| self.base.asm_manifests_mmr())
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

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.batch.epochal_writes_mut().cur_epoch = Some(epoch);
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.batch.epochal_writes_mut().asm_recorded_epoch = Some(epoch);
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.batch.epochal_writes_mut().total_ledger_balance = Some(amt);
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        // For append_manifest, we need to get the current MMR (from batch or
        // base), clone it, append, and store back.
        let mut mmr = self
            .batch
            .epochal_writes()
            .asm_manifests_mmr
            .clone()
            .unwrap_or_else(|| self.base.asm_manifests_mmr().clone());

        use strata_acct_types::{StrataHasher, tree_hash::TreeHash};
        let manifest_hash = <AsmManifest as TreeHash>::tree_hash_root(&mf);
        strata_merkle::Mmr::<StrataHasher>::add_leaf(&mut mmr, manifest_hash.into_inner())
            .expect("MMR capacity exceeded");

        let ew = self.batch.epochal_writes_mut();
        ew.asm_manifests_mmr = Some(mmr);
        ew.last_l1_blkid = Some(*mf.blkid());
        ew.last_l1_height = Some(height);
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
        let serial = self.next_account_serial();
        self.batch
            .ledger_mut()
            .create_account_from_data(id, new_acct_data, serial);
        Ok(serial)
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_asm_manifest_types::AsmManifest;
    use strata_identifiers::{Buf32, L1BlockId, L1Height, WtxidsRoot};
    use strata_ledger_types::*;
    use strata_ol_state_types::{IStateBatchApplicable, OLAccountState};

    use super::*;
    use crate::{
        batch_diff_layer::BatchDiffState, memory_state_layer::MemoryStateBaseLayer, test_utils::*,
    };

    // =========================================================================
    // Copy-on-write tests
    // =========================================================================

    #[test]
    fn test_read_falls_back_to_base() {
        let account_id = test_account_id(1);
        let (base_layer, serial) =
            setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let diff = BatchDiffState::new(&base_layer, &[]);
        let tracking = WriteTrackingState::new_empty(&diff);

        // Read should fall back to base since batch is empty
        let account = tracking.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));
    }

    #[test]
    fn test_check_account_exists_falls_back_to_base() {
        let account_id = test_account_id(1);
        let nonexistent_id = test_account_id(99);
        let (base_layer, _) =
            setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let diff = BatchDiffState::new(&base_layer, &[]);
        let tracking = WriteTrackingState::new_empty(&diff);

        assert!(tracking.check_account_exists(account_id).unwrap());
        assert!(!tracking.check_account_exists(nonexistent_id).unwrap());
    }

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

    #[test]
    fn test_read_prefers_batch_over_base() {
        let account_id = test_account_id(1);
        let (base_layer, _) =
            setup_layer_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        // Modify the account to put it in the batch
        tracking
            .update_account(account_id, |acct: &mut OLAccountState| {
                let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
                acct.add_balance(coin);
            })
            .unwrap();

        // Modify again - should use batch version
        tracking
            .update_account(account_id, |acct: &mut OLAccountState| {
                let coin = Coin::new_unchecked(BitcoinAmount::from_sat(100));
                acct.add_balance(coin);
            })
            .unwrap();

        // Final balance should be 1000 + 500 + 100 = 1600
        let account = tracking.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1600));
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

    #[test]
    fn test_find_account_id_by_serial_for_new_account() {
        let base_layer = create_test_base_layer();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        let account_id = test_account_id(1);
        let snark_state = test_snark_account_state(1);
        let new_acct = test_new_snark_account_data(&snark_state, BitcoinAmount::from_sat(5000));

        let serial = tracking.create_new_account(account_id, new_acct).unwrap();

        // Should be able to find the account by serial
        let found_id = tracking.find_account_id_by_serial(serial).unwrap();
        assert_eq!(found_id, Some(account_id));
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

    #[test]
    fn test_total_ledger_balance_in_batch() {
        let base_layer = create_test_base_layer();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        tracking.set_total_ledger_balance(BitcoinAmount::from_sat(1_000_000));

        assert_eq!(
            tracking.total_ledger_balance(),
            BitcoinAmount::from_sat(1_000_000)
        );
    }

    #[test]
    fn test_manifest_append_in_batch() {
        let base_layer = create_test_base_layer();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        let height = L1Height::from(100u32);
        let l1_blkid = L1BlockId::from(Buf32::from([1u8; 32]));
        let wtxids_root = WtxidsRoot::from(Buf32::from([2u8; 32]));
        let manifest =
            AsmManifest::new(height, l1_blkid, wtxids_root, vec![]).expect("valid test manifest");

        tracking.append_manifest(height, manifest);

        // The manifest should be recorded in the epochal state
        // (The actual validation of this would depend on the epochal state implementation)
    }

    // =========================================================================
    // State root tests
    // =========================================================================

    #[test]
    fn test_compute_state_root_no_writes() {
        let base_layer = create_test_base_layer();
        let base_root = base_layer.compute_state_root().unwrap();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let tracking = WriteTrackingState::new_empty(&diff);

        let result = tracking.compute_state_root();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), base_root);
    }

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
    // Error handling tests
    // =========================================================================

    #[test]
    fn test_update_nonexistent_account_returns_error() {
        let base_layer = create_test_base_layer();
        let diff = BatchDiffState::new(&base_layer, &[]);
        let mut tracking = WriteTrackingState::new_empty(&diff);

        let nonexistent_id = test_account_id(99);
        let result = tracking.update_account(nonexistent_id, |_acct: &mut OLAccountState| {});

        assert!(matches!(result, Err(StateError::MissingAccount(_))));
    }
}
