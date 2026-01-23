//! OL state layer that tracks writes to accumulators (MMRs) for indexing.
//!
//! This provides an `IStateAccessor` implementation that intercepts all writes
//! to accumulator structures (like MMRs) and records them for later use by
//! indexers, while passing all operations through to an inner implementation.

use std::fmt;

use strata_acct_types::{
    AccountId, AccountSerial, AccountTypeId, AcctResult, BitcoinAmount, Hash, Mmr64,
};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::{
    AccountTypeStateRef, Coin, IAccountState, IAccountStateMut, ISnarkAccountState,
    ISnarkAccountStateMut, IStateAccessor, NewAccountData,
};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::index_types::*;

// ============================================================================
// Snark account state wrapper (owned)
// ============================================================================

/// Wrapper around a snark account state that tracks `insert_inbox_message` calls.
///
/// This wrapper owns its inner state and an IndexerWrites buffer.
/// After modifications, use `into_parts()` to extract the inner state and writes.
pub struct IndexerSnarkAccountStateMut<S: ISnarkAccountStateMut> {
    inner: S,
    account_id: AccountId,
    writes: IndexerWrites,

    /// Tracks whether any modifications were made so we can decide if we want
    /// to bother writing back afterwards.  This might not be necessary anymore.
    modified: bool,
}

impl<S: ISnarkAccountStateMut + fmt::Debug> fmt::Debug for IndexerSnarkAccountStateMut<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexerSnarkAccountStateMut")
            .field("inner", &self.inner)
            .field("account_id", &self.account_id)
            .finish_non_exhaustive()
    }
}

impl<S: ISnarkAccountStateMut> IndexerSnarkAccountStateMut<S> {
    /// Creates a new wrapper.
    fn new(inner: S, account_id: AccountId) -> Self {
        Self {
            inner,
            account_id,
            writes: IndexerWrites::new(),
            modified: false,
        }
    }

    /// Returns whether this snark account was modified.
    pub fn was_modified(&self) -> bool {
        self.modified
    }

    /// Consumes the wrapper and returns the inner state, indexer writes,
    /// and whether the snark was modified.
    pub fn into_parts(self) -> (S, IndexerWrites, bool) {
        (self.inner, self.writes, self.modified)
    }
}

impl<S: ISnarkAccountStateMut + Clone> Clone for IndexerSnarkAccountStateMut<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            account_id: self.account_id,
            writes: self.writes.clone(),
            modified: self.modified,
        }
    }
}

impl<S: ISnarkAccountStateMut> ISnarkAccountState for IndexerSnarkAccountStateMut<S> {
    fn update_vk(&self) -> &PredicateKey {
        self.inner.update_vk()
    }

    fn seqno(&self) -> Seqno {
        self.inner.seqno()
    }

    fn inner_state_root(&self) -> Hash {
        self.inner.inner_state_root()
    }

    fn inbox_mmr(&self) -> &Mmr64 {
        self.inner.inbox_mmr()
    }

    fn next_inbox_msg_idx(&self) -> u64 {
        self.inner.next_inbox_msg_idx()
    }
}

impl<S: ISnarkAccountStateMut> ISnarkAccountStateMut for IndexerSnarkAccountStateMut<S> {
    fn set_proof_state_directly(&mut self, state: Hash, next_read_idx: u64, seqno: Seqno) {
        let op = SAStateSetOp::new(self.account_id, state, next_read_idx, seqno);

        // Pass through to inner.
        self.inner
            .set_proof_state_directly(state, next_read_idx, seqno);
        self.modified = true;

        // Track the write.
        self.writes
            .push_snark_acct_update(SnarkAcctStateUpdate::DirectSet(op));
    }

    fn update_inner_state(
        &mut self,
        inner_state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
        extra_data: &[u8],
    ) -> AcctResult<()> {
        let op = SAStateUpdateOp::new(
            self.account_id,
            inner_state,
            next_read_idx,
            seqno,
            extra_data.to_vec(),
        );

        // Pass through to inner first, if this fails we won't have emitted
        // anything.
        self.inner
            .update_inner_state(inner_state, next_read_idx, seqno, extra_data)?;
        self.modified = true;

        // Track the write.
        self.writes
            .push_snark_acct_update(SnarkAcctStateUpdate::Update(op));

        Ok(())
    }

    fn insert_inbox_message(&mut self, entry: MessageEntry) -> AcctResult<()> {
        // Record the index BEFORE insertion so that we capture the correct index.
        let index = self.inner.inbox_mmr().num_entries();
        let entry2 = entry.clone();

        // Pass through to inner.
        self.inner.insert_inbox_message(entry)?;
        self.modified = true;

        // Only emit the write if it was successful.
        self.writes
            .push_inbox_message(InboxMessageWrite::new(self.account_id, entry2, index));

        Ok(())
    }
}

// ============================================================================
// Account state wrapper (owned)
// ============================================================================

/// Wrapper around an account state that tracks inbox MMR writes.
///
/// This wrapper owns its inner state and an IndexerWrites buffer.
/// After modifications, use `into_parts()` to extract the inner state and writes.
pub struct IndexerAccountStateMut<A: IAccountStateMut> {
    inner: A,
    account_id: AccountId,
    writes: IndexerWrites,

    /// Tracks whether any modifications were made to this account.
    modified: bool,

    /// Cached snark wrapper, lazily initialized.
    snark_wrapper: Option<IndexerSnarkAccountStateMut<A::SnarkAccountStateMut>>,
}

impl<A: IAccountStateMut + fmt::Debug> fmt::Debug for IndexerAccountStateMut<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexerAccountStateMut")
            .field("inner", &self.inner)
            .field("account_id", &self.account_id)
            .finish_non_exhaustive()
    }
}

impl<A: IAccountStateMut> IndexerAccountStateMut<A> {
    /// Creates a new wrapper.
    pub fn new(inner: A, account_id: AccountId) -> Self {
        Self {
            inner,
            account_id,
            writes: IndexerWrites::new(),
            modified: false,
            snark_wrapper: None,
        }
    }

    /// Returns whether this account was modified.
    pub fn was_modified(&self) -> bool {
        self.modified
            || self
                .snark_wrapper
                .as_ref()
                .is_some_and(|s| s.was_modified())
    }

    /// Consumes the wrapper and returns the inner state, accumulated writes,
    /// and whether the account was modified.
    ///
    /// If a snark wrapper was created and modified, its state is synced back
    /// to the inner account.
    pub fn into_parts(mut self) -> (A, IndexerWrites, bool) {
        let mut modified = self.modified;

        // If we have a snark wrapper, check if it was modified
        if let Some(snark_wrapper) = self.snark_wrapper.take() {
            let (snark_inner, snark_writes, snark_modified) = snark_wrapper.into_parts();
            self.writes.extend(snark_writes);

            // If the snark was modified, sync it back to the inner account
            if snark_modified {
                modified = true;
                // We need to get a mutable reference to the inner's snark state
                // and update it with our modified copy
                if let Ok(inner_snark) = self.inner.as_snark_account_mut() {
                    *inner_snark = snark_inner;
                }
            }
        }

        (self.inner, self.writes, modified)
    }
}

// idk why we need this separately
impl<A: IAccountStateMut + Clone> Clone for IndexerAccountStateMut<A>
where
    A::SnarkAccountStateMut: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            account_id: self.account_id,
            writes: self.writes.clone(),
            modified: self.modified,
            snark_wrapper: self.snark_wrapper.clone(),
        }
    }
}

impl<A: IAccountStateMut> IAccountState for IndexerAccountStateMut<A> {
    type SnarkAccountState = A::SnarkAccountState;

    fn serial(&self) -> AccountSerial {
        self.inner.serial()
    }

    fn balance(&self) -> BitcoinAmount {
        self.inner.balance()
    }

    fn ty(&self) -> AccountTypeId {
        self.inner.ty()
    }

    fn type_state(&self) -> AccountTypeStateRef<'_, Self> {
        match self.inner.type_state() {
            AccountTypeStateRef::Empty => AccountTypeStateRef::Empty,
            AccountTypeStateRef::Snark(s) => AccountTypeStateRef::Snark(s),
        }
    }

    fn as_snark_account(&self) -> AcctResult<&Self::SnarkAccountState> {
        self.inner.as_snark_account()
    }
}

impl<A: IAccountStateMut> IAccountStateMut for IndexerAccountStateMut<A>
where
    A::SnarkAccountStateMut: Clone,
{
    type SnarkAccountStateMut = IndexerSnarkAccountStateMut<A::SnarkAccountStateMut>;

    fn add_balance(&mut self, coin: Coin) {
        self.modified = true;
        self.inner.add_balance(coin);
    }

    fn take_balance(&mut self, amt: BitcoinAmount) -> AcctResult<Coin> {
        self.modified = true;
        self.inner.take_balance(amt)
    }

    fn as_snark_account_mut(&mut self) -> AcctResult<&mut Self::SnarkAccountStateMut> {
        // Initialize the snark wrapper lazily if needed.
        // We clone the snark state so we can own it in our wrapper while still
        // being able to sync changes back to the inner account in into_parts().
        if self.snark_wrapper.is_none() {
            let inner_snark = self.inner.as_snark_account_mut()?.clone();
            self.snark_wrapper = Some(IndexerSnarkAccountStateMut::new(
                inner_snark,
                self.account_id,
            ));
        }
        Ok(self.snark_wrapper.as_mut().unwrap())
    }
}

// ============================================================================
// Main state accessor wrapper
// ============================================================================

/// A state accessor wrapper that tracks writes to accumulators.
///
/// This wrapper intercepts all writes to MMRs and other accumulator structures,
/// recording them for later use by indexers. All operations are passed through
/// to the inner implementation.
pub struct IndexerState<S: IStateAccessor> {
    inner: S,
    writes: IndexerWrites,
}

impl<S: IStateAccessor + fmt::Debug> fmt::Debug for IndexerState<S>
where
    S::AccountState: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexerState")
            .field("inner", &self.inner)
            .field("writes", &self.writes)
            .finish()
    }
}

impl<S: IStateAccessor> IndexerState<S> {
    /// Creates a new indexer state wrapping the given inner state.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            writes: IndexerWrites::new(),
        }
    }

    /// Returns a reference to the tracked accumulator writes.
    pub fn writes(&self) -> &IndexerWrites {
        &self.writes
    }

    /// Consumes this wrapper and returns the inner state and tracked writes.
    pub fn into_parts(self) -> (S, IndexerWrites) {
        (self.inner, self.writes)
    }

    /// Returns a reference to the inner state.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Returns a mutable reference to the inner state.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }
}

impl<S: IStateAccessor> IStateAccessor for IndexerState<S>
where
    S::AccountStateMut: Clone,
    <S::AccountStateMut as IAccountStateMut>::SnarkAccountStateMut: Clone,
{
    type AccountState = S::AccountState;
    type AccountStateMut = IndexerAccountStateMut<S::AccountStateMut>;

    // ===== Global state methods (pass through) =====

    fn cur_slot(&self) -> u64 {
        self.inner.cur_slot()
    }

    fn set_cur_slot(&mut self, slot: u64) {
        self.inner.set_cur_slot(slot);
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.inner.cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.inner.set_cur_epoch(epoch);
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.inner.last_l1_blkid()
    }

    fn last_l1_height(&self) -> L1Height {
        self.inner.last_l1_height()
    }

    fn append_manifest(&mut self, height: L1Height, mf: AsmManifest) {
        // Track the manifest write
        self.writes.push_manifest(ManifestWrite {
            height,
            manifest: mf.clone(),
        });
        // Pass through to inner
        self.inner.append_manifest(height, mf);
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.inner.asm_recorded_epoch()
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.inner.set_asm_recorded_epoch(epoch);
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.inner.total_ledger_balance()
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.inner.set_total_ledger_balance(amt);
    }

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        self.inner.asm_manifests_mmr()
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        self.inner.check_account_exists(id)
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        self.inner.get_account_state(id)
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> AcctResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        // Clone the account state from inner, wrap it, let user modify, then write back
        let (result, local_writes) = self.inner.update_account(id, |inner_acct| {
            // Clone the inner account and wrap it
            let mut wrapped = IndexerAccountStateMut::new(inner_acct.clone(), id);

            // Let the user modify the wrapped version
            let user_result = f(&mut wrapped);

            // Extract the modified inner state, writes, and modification flag
            let (modified_inner, writes, was_modified) = wrapped.into_parts();

            // Only write back if actually modified
            if was_modified {
                *inner_acct = modified_inner;
            }

            (user_result, writes)
        })?;

        // Merge local writes into our accumulator
        self.writes.extend(local_writes);
        Ok(result)
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        self.inner.create_new_account(id, new_acct_data)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        self.inner.find_account_id_by_serial(serial)
    }

    fn next_account_serial(&self) -> AccountSerial {
        self.inner.next_account_serial()
    }

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        self.inner.compute_state_root()
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_asm_manifest_types::AsmManifest;
    use strata_identifiers::{Buf32, L1BlockId, L1Height, WtxidsRoot};
    use strata_ledger_types::{
        AccountTypeState, Coin, IAccountState, IAccountStateMut, IStateAccessor, NewAccountData,
    };
    use strata_ol_state_types::OLState;
    use strata_snark_acct_types::Seqno;

    use super::*;
    use crate::test_utils::*;

    // =========================================================================
    // Pass-through tests
    // =========================================================================

    #[test]
    fn test_passthrough_slot() {
        let state = OLState::new_genesis();
        let mut indexer = IndexerState::new(state);

        // Test initial slot
        assert_eq!(indexer.cur_slot(), 0);

        // Test setting slot
        indexer.set_cur_slot(42);
        assert_eq!(indexer.cur_slot(), 42);

        // Verify inner state was updated
        let (inner, _) = indexer.into_parts();
        assert_eq!(inner.cur_slot(), 42);
    }

    #[test]
    fn test_passthrough_epoch() {
        let state = OLState::new_genesis();
        let mut indexer = IndexerState::new(state);

        // Test initial epoch
        assert_eq!(indexer.cur_epoch(), 0);

        // Test setting epoch
        indexer.set_cur_epoch(5);
        assert_eq!(indexer.cur_epoch(), 5);

        // Verify inner state was updated
        let (inner, _) = indexer.into_parts();
        assert_eq!(inner.cur_epoch(), 5);
    }

    #[test]
    fn test_passthrough_get_account_state() {
        let account_id = test_account_id(1);
        let (state, serial) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let indexer = IndexerState::new(state);

        // Verify account can be retrieved
        let account = indexer.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));
    }

    #[test]
    fn test_passthrough_check_account_exists() {
        let account_id = test_account_id(1);
        let nonexistent_id = test_account_id(99);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let indexer = IndexerState::new(state);

        assert!(indexer.check_account_exists(account_id).unwrap());
        assert!(!indexer.check_account_exists(nonexistent_id).unwrap());
    }

    #[test]
    fn test_passthrough_create_account() {
        let state = OLState::new_genesis();
        let mut indexer = IndexerState::new(state);

        let account_id = test_account_id(1);
        let snark_state = test_snark_account_state(1);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(5000),
            AccountTypeState::Snark(snark_state),
        );

        let serial = indexer.create_new_account(account_id, new_acct).unwrap();

        // Verify account was created
        assert!(indexer.check_account_exists(account_id).unwrap());
        let account = indexer.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);
        assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));
    }

    #[test]
    fn test_passthrough_compute_state_root() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

        // Get state root directly
        let direct_root = state.compute_state_root().unwrap();

        // Get state root through indexer
        let indexer = IndexerState::new(state);
        let indexer_root = indexer.compute_state_root().unwrap();

        assert_eq!(direct_root, indexer_root);
    }

    // =========================================================================
    // Write tracking tests
    // =========================================================================

    #[test]
    fn test_tracks_inbox_message_writes() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Insert a message into the inbox
        let msg = test_message_entry(50, 0, 2000);
        indexer
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .insert_inbox_message(msg.clone())
            })
            .unwrap()
            .unwrap();

        // Verify the write was tracked
        let (_, writes) = indexer.into_parts();
        assert_eq!(writes.inbox_messages().len(), 1);
        assert_eq!(writes.inbox_messages()[0].account_id, account_id);
        assert_eq!(writes.inbox_messages()[0].index, 0); // First message at index 0
    }

    #[test]
    fn test_tracks_multiple_inbox_writes_same_account() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Insert multiple messages
        for i in 0..3 {
            let msg = test_message_entry(i, 0, (i as u64 + 1) * 1000);
            indexer
                .update_account(account_id, |acct| {
                    acct.as_snark_account_mut()
                        .unwrap()
                        .insert_inbox_message(msg.clone())
                })
                .unwrap()
                .unwrap();
        }

        // Verify all writes were tracked
        let (_, writes) = indexer.into_parts();
        assert_eq!(writes.inbox_messages().len(), 3);

        // Verify indices are sequential
        for (i, write) in writes.inbox_messages().iter().enumerate() {
            assert_eq!(write.index, i as u64);
            assert_eq!(write.account_id, account_id);
        }
    }

    #[test]
    fn test_tracks_writes_across_accounts() {
        let account_id_1 = test_account_id(1);
        let account_id_2 = test_account_id(2);

        // Setup state with two snark accounts
        let mut state = OLState::new_genesis();
        let snark_state_1 = test_snark_account_state(1);
        let snark_state_2 = test_snark_account_state(2);
        state
            .create_new_account(
                account_id_1,
                NewAccountData::new(
                    BitcoinAmount::from_sat(1000),
                    AccountTypeState::Snark(snark_state_1),
                ),
            )
            .unwrap();
        state
            .create_new_account(
                account_id_2,
                NewAccountData::new(
                    BitcoinAmount::from_sat(2000),
                    AccountTypeState::Snark(snark_state_2),
                ),
            )
            .unwrap();

        let mut indexer = IndexerState::new(state);

        // Insert message to first account
        let msg1 = test_message_entry(10, 0, 1000);
        indexer
            .update_account(account_id_1, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .insert_inbox_message(msg1.clone())
            })
            .unwrap()
            .unwrap();

        // Insert message to second account
        let msg2 = test_message_entry(20, 0, 2000);
        indexer
            .update_account(account_id_2, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .insert_inbox_message(msg2.clone())
            })
            .unwrap()
            .unwrap();

        // Verify writes for both accounts
        let (_, writes) = indexer.into_parts();
        assert_eq!(writes.inbox_messages().len(), 2);

        // First write should be for account 1
        assert_eq!(writes.inbox_messages()[0].account_id, account_id_1);
        assert_eq!(writes.inbox_messages()[0].index, 0);

        // Second write should be for account 2
        assert_eq!(writes.inbox_messages()[1].account_id, account_id_2);
        assert_eq!(writes.inbox_messages()[1].index, 0);
    }

    #[test]
    fn test_tracks_manifest_writes() {
        let state = OLState::new_genesis();
        let mut indexer = IndexerState::new(state);

        // Create a test manifest
        let height = L1Height::from(100u32);
        let l1_blkid = L1BlockId::from(Buf32::from([1u8; 32]));
        let wtxids_root = WtxidsRoot::from(Buf32::from([2u8; 32]));
        let manifest = AsmManifest::new(height.into(), l1_blkid, wtxids_root, vec![]);

        // Append the manifest
        indexer.append_manifest(height, manifest.clone());

        // Verify the write was tracked
        let (_, writes) = indexer.into_parts();
        assert_eq!(writes.manifests().len(), 1);
        assert_eq!(writes.manifests()[0].height, height);
    }

    // =========================================================================
    // Modification flag tests
    // =========================================================================

    #[test]
    fn test_modification_flag_on_balance_add() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Add balance
        indexer
            .update_account(account_id, |acct| {
                let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
                acct.add_balance(coin);
            })
            .unwrap();

        // Verify the balance was actually updated in inner state
        let (inner, _) = indexer.into_parts();
        let account = inner.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(1500));
    }

    #[test]
    fn test_modification_flag_on_balance_take() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Take balance
        indexer
            .update_account(account_id, |acct| {
                let coin = acct.take_balance(BitcoinAmount::from_sat(300)).unwrap();
                coin.safely_consume_unchecked();
            })
            .unwrap();

        // Verify the balance was actually updated in inner state
        let (inner, _) = indexer.into_parts();
        let account = inner.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.balance(), BitcoinAmount::from_sat(700));
    }

    #[test]
    fn test_modification_flag_on_snark_update() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Update snark state
        let new_hash = test_hash(99);
        indexer
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .set_proof_state_directly(new_hash, 0, Seqno::from(1));
            })
            .unwrap();

        // Verify the snark state was updated
        let (inner, _) = indexer.into_parts();
        let account = inner.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(
            account.as_snark_account().unwrap().inner_state_root(),
            new_hash
        );
    }

    #[test]
    fn test_no_modification_when_closure_doesnt_mutate() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let original_root = state.compute_state_root().unwrap();
        let mut indexer = IndexerState::new(state);

        // Call update_account but don't actually modify anything
        indexer
            .update_account(account_id, |acct| {
                // Just read the balance, don't modify
                let _ = acct.balance();
            })
            .unwrap();

        // Verify no writes were tracked (at least for inbox)
        let (inner, writes) = indexer.into_parts();
        assert!(writes.inbox_messages().is_empty());

        // State root should be unchanged
        assert_eq!(inner.compute_state_root().unwrap(), original_root);
    }

    // =========================================================================
    // State consistency tests (direct vs wrapped)
    // =========================================================================

    #[test]
    fn test_direct_vs_wrapped_inbox_insert() {
        let account_id = test_account_id(1);
        let balance = BitcoinAmount::from_sat(1000);

        // Create two identical states
        let (mut direct_state, _) = setup_state_with_snark_account(account_id, 1, balance);
        let (base_state, _) = setup_state_with_snark_account(account_id, 1, balance);
        let mut wrapped_state = IndexerState::new(base_state);

        // Create identical message
        let msg = test_message_entry(50, 0, 2000);

        // Apply to direct state
        direct_state
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .insert_inbox_message(msg.clone())
            })
            .unwrap()
            .unwrap();

        // Apply to wrapped state
        wrapped_state
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .insert_inbox_message(msg.clone())
            })
            .unwrap()
            .unwrap();

        // Extract inner state from wrapper
        let (inner_state, writes) = wrapped_state.into_parts();

        // Compare account states
        let direct_acct = direct_state.get_account_state(account_id).unwrap().unwrap();
        let wrapped_acct = inner_state.get_account_state(account_id).unwrap().unwrap();

        assert_eq!(direct_acct.balance(), wrapped_acct.balance());
        assert_eq!(
            direct_acct
                .as_snark_account()
                .unwrap()
                .inbox_mmr()
                .num_entries(),
            wrapped_acct
                .as_snark_account()
                .unwrap()
                .inbox_mmr()
                .num_entries()
        );

        // Verify writes were tracked
        assert_eq!(writes.inbox_messages().len(), 1);
        assert_eq!(writes.inbox_messages()[0].index, 0);
    }

    #[test]
    fn test_direct_vs_wrapped_balance_update() {
        let account_id = test_account_id(1);
        let balance = BitcoinAmount::from_sat(1000);

        // Create two identical states
        let (mut direct_state, _) = setup_state_with_snark_account(account_id, 1, balance);
        let (base_state, _) = setup_state_with_snark_account(account_id, 1, balance);
        let mut wrapped_state = IndexerState::new(base_state);

        // Apply balance change to both
        let add_amount = BitcoinAmount::from_sat(500);

        direct_state
            .update_account(account_id, |acct| {
                let coin = Coin::new_unchecked(add_amount);
                acct.add_balance(coin);
            })
            .unwrap();

        wrapped_state
            .update_account(account_id, |acct| {
                let coin = Coin::new_unchecked(add_amount);
                acct.add_balance(coin);
            })
            .unwrap();

        // Extract inner state from wrapper
        let (inner_state, _) = wrapped_state.into_parts();

        // Compare balances
        let direct_acct = direct_state.get_account_state(account_id).unwrap().unwrap();
        let wrapped_acct = inner_state.get_account_state(account_id).unwrap().unwrap();

        assert_eq!(direct_acct.balance(), wrapped_acct.balance());
        assert_eq!(wrapped_acct.balance(), BitcoinAmount::from_sat(1500));
    }

    // =========================================================================
    // Write data accuracy tests
    // =========================================================================

    #[test]
    fn test_inbox_write_captures_pre_insertion_index() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Insert three messages sequentially
        for i in 0..3 {
            let msg = test_message_entry(i, 0, (i as u64 + 1) * 1000);
            indexer
                .update_account(account_id, |acct| {
                    acct.as_snark_account_mut()
                        .unwrap()
                        .insert_inbox_message(msg.clone())
                })
                .unwrap()
                .unwrap();
        }

        let (_, writes) = indexer.into_parts();

        // Verify indices are the BEFORE-insertion indices (0, 1, 2)
        assert_eq!(writes.inbox_messages()[0].index, 0);
        assert_eq!(writes.inbox_messages()[1].index, 1);
        assert_eq!(writes.inbox_messages()[2].index, 2);
    }

    #[test]
    fn test_inbox_write_captures_correct_account_id() {
        let account_id = test_account_id(42);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        let msg = test_message_entry(1, 0, 1000);
        indexer
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .insert_inbox_message(msg.clone())
            })
            .unwrap()
            .unwrap();

        let (_, writes) = indexer.into_parts();
        assert_eq!(writes.inbox_messages()[0].account_id, account_id);
    }

    #[test]
    fn test_writes_empty_initially() {
        let state = OLState::new_genesis();
        let indexer = IndexerState::new(state);

        assert!(indexer.writes().is_empty());
        assert!(indexer.writes().inbox_messages().is_empty());
        assert!(indexer.writes().manifests().is_empty());
    }

    #[test]
    fn test_into_parts_returns_inner_and_writes() {
        let account_id = test_account_id(1);
        let (state, serial) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Make a modification
        let msg = test_message_entry(1, 0, 1000);
        indexer
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .insert_inbox_message(msg.clone())
            })
            .unwrap()
            .unwrap();

        let (inner, writes) = indexer.into_parts();

        // Verify inner state is intact
        let account = inner.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(account.serial(), serial);

        // Verify writes were collected
        assert_eq!(writes.inbox_messages().len(), 1);
    }

    // =========================================================================
    // Snark state update tracking tests
    // =========================================================================

    #[test]
    fn test_tracks_direct_set() {
        use crate::SnarkAcctStateUpdate;

        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Update proof state directly
        let new_hash = test_hash(42);
        let next_read_idx = 5;
        let seqno = Seqno::from(10);
        indexer
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .set_proof_state_directly(new_hash, next_read_idx, seqno);
            })
            .unwrap();

        // Verify the write was tracked
        let (_, writes) = indexer.into_parts();
        assert_eq!(writes.snark_state_updates().len(), 1);

        match &writes.snark_state_updates()[0] {
            SnarkAcctStateUpdate::DirectSet(s) => {
                assert_eq!(s.account_id(), account_id);
                assert_eq!(s.state(), new_hash.0);
                assert_eq!(s.next_read_idx(), next_read_idx);
                assert_eq!(s.seqno(), seqno);
            }
            _ => panic!("expected DirectSet variant"),
        }
    }

    #[test]
    fn test_tracks_inner_state_updates() {
        use crate::SnarkAcctStateUpdate;

        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Update inner state
        let inner_state = test_hash(99);
        let next_read_idx = 3;
        let seqno = Seqno::from(7);
        let extra_data = vec![1, 2, 3, 4, 5];
        indexer
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut().unwrap().update_inner_state(
                    inner_state,
                    next_read_idx,
                    seqno,
                    &extra_data,
                )
            })
            .unwrap()
            .unwrap();

        // Verify the write was tracked
        let (_, writes) = indexer.into_parts();
        assert_eq!(writes.snark_state_updates().len(), 1);

        match &writes.snark_state_updates()[0] {
            SnarkAcctStateUpdate::Update(s) => {
                assert_eq!(s.account_id(), account_id);
                assert_eq!(s.inner_state(), inner_state.0);
                assert_eq!(s.next_read_idx(), next_read_idx);
                assert_eq!(s.seqno(), seqno);
                assert_eq!(s.extra_data(), extra_data);
            }
            _ => panic!("expected Update variant"),
        }
    }

    #[test]
    fn test_tracks_multiple_snark_state_updates() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Multiple proof state updates
        for i in 0..3 {
            let hash = test_hash(i);
            indexer
                .update_account(account_id, |acct| {
                    acct.as_snark_account_mut()
                        .unwrap()
                        .set_proof_state_directly(hash, i as u64, Seqno::from(i as u64));
                })
                .unwrap();
        }

        // Verify all writes were tracked
        let (_, writes) = indexer.into_parts();
        assert_eq!(writes.snark_state_updates().len(), 3);

        for (i, update) in writes.snark_state_updates().iter().enumerate() {
            assert_eq!(update.account_id(), account_id);
            assert_eq!(update.next_read_idx(), i as u64);
            assert_eq!(update.seqno(), Seqno::from(i as u64));
        }
    }

    #[test]
    fn test_tracks_state_updates_across_accounts() {
        use crate::SnarkAcctStateUpdate;

        let account_id_1 = test_account_id(1);
        let account_id_2 = test_account_id(2);

        // Setup state with two snark accounts
        let mut state = OLState::new_genesis();
        let snark_state_1 = test_snark_account_state(1);
        let snark_state_2 = test_snark_account_state(2);
        state
            .create_new_account(
                account_id_1,
                NewAccountData::new(
                    BitcoinAmount::from_sat(1000),
                    AccountTypeState::Snark(snark_state_1),
                ),
            )
            .unwrap();
        state
            .create_new_account(
                account_id_2,
                NewAccountData::new(
                    BitcoinAmount::from_sat(2000),
                    AccountTypeState::Snark(snark_state_2),
                ),
            )
            .unwrap();

        let mut indexer = IndexerState::new(state);

        // Update proof state for first account (DirectSet)
        indexer
            .update_account(account_id_1, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .set_proof_state_directly(test_hash(1), 0, Seqno::from(1));
            })
            .unwrap();

        // Update inner state for second account (Update)
        indexer
            .update_account(account_id_2, |acct| {
                acct.as_snark_account_mut().unwrap().update_inner_state(
                    test_hash(2),
                    0,
                    Seqno::from(1),
                    &[10, 20, 30],
                )
            })
            .unwrap()
            .unwrap();

        // Verify writes for both accounts
        let (_, writes) = indexer.into_parts();
        assert_eq!(writes.snark_state_updates().len(), 2);

        // First should be DirectSet for account_id_1
        match &writes.snark_state_updates()[0] {
            SnarkAcctStateUpdate::DirectSet(s) => {
                assert_eq!(s.account_id(), account_id_1);
            }
            _ => panic!("expected DirectSet variant"),
        }

        // Second should be Update for account_id_2
        match &writes.snark_state_updates()[1] {
            SnarkAcctStateUpdate::Update(s) => {
                assert_eq!(s.account_id(), account_id_2);
                assert_eq!(s.extra_data(), vec![10, 20, 30]);
            }
            _ => panic!("expected Update variant"),
        }
    }

    #[test]
    fn test_is_empty_includes_state_updates() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Initially empty
        assert!(indexer.writes().is_empty());

        // Add a proof state write
        indexer
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .set_proof_state_directly(test_hash(1), 0, Seqno::from(1));
            })
            .unwrap();

        // No longer empty
        let (_, writes) = indexer.into_parts();
        assert!(!writes.is_empty());
    }

    #[test]
    fn test_state_update_captures_inner_state_change() {
        let account_id = test_account_id(1);
        let (state, _) =
            setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
        let mut indexer = IndexerState::new(state);

        // Update proof state
        let new_hash = test_hash(77);
        indexer
            .update_account(account_id, |acct| {
                acct.as_snark_account_mut()
                    .unwrap()
                    .set_proof_state_directly(new_hash, 0, Seqno::from(1));
            })
            .unwrap();

        // Verify the inner state was actually updated
        let (inner, _) = indexer.into_parts();
        let account = inner.get_account_state(account_id).unwrap().unwrap();
        assert_eq!(
            account.as_snark_account().unwrap().inner_state_root(),
            new_hash
        );
    }
}
