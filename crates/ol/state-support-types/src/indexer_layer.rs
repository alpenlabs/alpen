//! OL state layer that tracks writes to accumulators (MMRs) for indexing.
//!
//! This provides an `IStateAccessor` implementation that intercepts all writes
//! to accumulator structures (like MMRs) and records them for later use by
//! indexers, while passing all operations through to an inner implementation.

use strata_acct_types::{
    AccountId, AccountSerial, AccountTypeId, AcctResult, BitcoinAmount, Hash, Mmr64,
};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::{
    AccountTypeStateRef, Coin, IAccountState, IAccountStateMut, ISnarkAccountState,
    ISnarkAccountStateMut, IStateAccessor, NewAccountData,
};
use strata_snark_acct_types::{MessageEntry, Seqno};

// ============================================================================
// Tracked write types
// ============================================================================

/// A tracked inbox message write.
#[derive(Clone, Debug)]
pub struct InboxMessageWrite {
    /// The account that received the message.
    pub account_id: AccountId,

    /// The message entry that was inserted.
    pub entry: MessageEntry,

    /// The index in the MMR where this entry was inserted.
    pub index: u64,
}

/// A tracked manifest write.
#[derive(Clone, Debug)]
pub struct ManifestWrite {
    /// The L1 block height associated with the manifest.
    pub height: L1Height,

    /// The manifest that was appended.
    pub manifest: AsmManifest,
}

/// Collection of all tracked accumulator writes.
///
/// This struct is extensible - add new `Vec` fields for future MMR types.
#[derive(Clone, Debug, Default)]
pub struct AccumulatorWrites {
    inbox_messages: Vec<InboxMessageWrite>,
    manifests: Vec<ManifestWrite>,
}

impl AccumulatorWrites {
    /// Creates a new empty collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an inbox message write.
    pub fn push_inbox_message(&mut self, account_id: AccountId, entry: MessageEntry, index: u64) {
        self.inbox_messages.push(InboxMessageWrite {
            account_id,
            entry,
            index,
        });
    }

    /// Records a manifest write.
    pub fn push_manifest(&mut self, height: L1Height, manifest: AsmManifest) {
        self.manifests.push(ManifestWrite { height, manifest });
    }

    /// Returns all tracked inbox message writes.
    pub fn inbox_messages(&self) -> &[InboxMessageWrite] {
        &self.inbox_messages
    }

    /// Returns all tracked manifest writes.
    pub fn manifests(&self) -> &[ManifestWrite] {
        &self.manifests
    }

    /// Returns true if no writes have been tracked.
    pub fn is_empty(&self) -> bool {
        self.inbox_messages.is_empty() && self.manifests.is_empty()
    }

    /// Extends this collection with writes from another.
    pub fn extend(&mut self, other: AccumulatorWrites) {
        self.inbox_messages.extend(other.inbox_messages);
        self.manifests.extend(other.manifests);
    }
}

// ============================================================================
// Snark account state wrapper (owned)
// ============================================================================

/// Wrapper around a snark account state that tracks `insert_inbox_message` calls.
///
/// This wrapper owns its inner state and an AccumulatorWrites buffer.
/// After modifications, use `into_parts()` to extract the inner state and writes.
pub struct IndexerSnarkAccountStateMut<S: ISnarkAccountStateMut> {
    inner: S,
    account_id: AccountId,
    writes: AccumulatorWrites,
    /// Tracks whether any modifications were made.
    modified: bool,
}

impl<S: ISnarkAccountStateMut + std::fmt::Debug> std::fmt::Debug
    for IndexerSnarkAccountStateMut<S>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            writes: AccumulatorWrites::new(),
            modified: false,
        }
    }

    /// Returns whether this snark account was modified.
    pub fn was_modified(&self) -> bool {
        self.modified
    }

    /// Consumes the wrapper and returns the inner state, accumulated writes,
    /// and whether the snark was modified.
    pub fn into_parts(self) -> (S, AccumulatorWrites, bool) {
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
    fn seqno(&self) -> Seqno {
        self.inner.seqno()
    }

    fn inner_state_root(&self) -> Hash {
        self.inner.inner_state_root()
    }

    fn inbox_mmr(&self) -> &Mmr64 {
        self.inner.inbox_mmr()
    }
}

impl<S: ISnarkAccountStateMut> ISnarkAccountStateMut for IndexerSnarkAccountStateMut<S> {
    fn set_proof_state_directly(&mut self, state: Hash, next_read_idx: u64, seqno: Seqno) {
        self.modified = true;
        self.inner
            .set_proof_state_directly(state, next_read_idx, seqno);
    }

    fn update_inner_state(
        &mut self,
        inner_state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
        extra_data: &[u8],
    ) -> AcctResult<()> {
        self.modified = true;
        self.inner
            .update_inner_state(inner_state, next_read_idx, seqno, extra_data)
    }

    fn insert_inbox_message(&mut self, entry: MessageEntry) -> AcctResult<()> {
        self.modified = true;
        // Record the write BEFORE insertion so we capture the correct index
        let index = self.inner.inbox_mmr().num_entries();
        self.writes
            .push_inbox_message(self.account_id, entry.clone(), index);

        // Pass through to inner
        self.inner.insert_inbox_message(entry)
    }
}

// ============================================================================
// Account state wrapper (owned)
// ============================================================================

/// Wrapper around an account state that tracks inbox MMR writes.
///
/// This wrapper owns its inner state and an AccumulatorWrites buffer.
/// After modifications, use `into_parts()` to extract the inner state and writes.
pub struct IndexerAccountStateMut<A: IAccountStateMut> {
    inner: A,
    account_id: AccountId,
    writes: AccumulatorWrites,
    /// Tracks whether any modifications were made to this account.
    modified: bool,
    /// Cached snark wrapper, lazily initialized.
    snark_wrapper: Option<IndexerSnarkAccountStateMut<A::SnarkAccountStateMut>>,
}

impl<A: IAccountStateMut + std::fmt::Debug> std::fmt::Debug for IndexerAccountStateMut<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexerAccountStateMut")
            .field("inner", &self.inner)
            .field("account_id", &self.account_id)
            .finish_non_exhaustive()
    }
}

impl<A: IAccountStateMut> IndexerAccountStateMut<A> {
    /// Creates a new wrapper.
    fn new(inner: A, account_id: AccountId) -> Self {
        Self {
            inner,
            account_id,
            writes: AccumulatorWrites::new(),
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
                .map_or(false, |s| s.was_modified())
    }

    /// Consumes the wrapper and returns the inner state, accumulated writes,
    /// and whether the account was modified.
    ///
    /// If a snark wrapper was created and modified, its state is synced back
    /// to the inner account.
    pub fn into_parts(mut self) -> (A, AccumulatorWrites, bool) {
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
    writes: AccumulatorWrites,
}

impl<S: IStateAccessor + std::fmt::Debug> std::fmt::Debug for IndexerState<S>
where
    S::AccountState: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            writes: AccumulatorWrites::new(),
        }
    }

    /// Returns a reference to the tracked accumulator writes.
    pub fn writes(&self) -> &AccumulatorWrites {
        &self.writes
    }

    /// Consumes this wrapper and returns the inner state and tracked writes.
    pub fn into_parts(self) -> (S, AccumulatorWrites) {
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
        self.writes.push_manifest(height, mf.clone());
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

    fn compute_state_root(&self) -> AcctResult<Buf32> {
        self.inner.compute_state_root()
    }
}
