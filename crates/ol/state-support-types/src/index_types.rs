//! Types for representing collected index data from state operations.
//!
//! This module contains types that capture operations performed on state
//! for later use by indexers. These are produced by the `IndexerState` layer.
// TODO(STR-3677): make the field names here more consistent, which should also reflect in
// the spec and state accessor fn/arg names

use strata_acct_types::{AccountId, Hash, L1BlockRecord, MessageEntry};
use strata_identifiers::L1Height;
use strata_predicate::PredicateKey;
use strata_snark_acct_types::Seqno;

// ============================================================================
// Inbox message tracking
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

impl InboxMessageWrite {
    pub fn new(account_id: AccountId, entry: MessageEntry, index: u64) -> Self {
        Self {
            account_id,
            entry,
            index,
        }
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    pub fn entry(&self) -> &MessageEntry {
        &self.entry
    }

    pub fn index(&self) -> u64 {
        self.index
    }
}

// ============================================================================
// Snark state update tracking
// ============================================================================

/// A tracked snark account state update, recorded for each `set_proof_state` call.
///
/// Extra data associated with the update is no longer tracked here; it is sourced from the
/// emitted `SnarkAccountUpdateLogData` logs at indexing time.
#[derive(Clone, Debug)]
pub struct SnarkAcctStateUpdate {
    /// The account whose state was updated.
    account_id: AccountId,

    /// The new inner state root, if known.
    ///
    /// Present on block-sync updates. On checkpoint-sync only the terminal
    /// per-account update of an epoch carries a root (the recoverable
    /// post-epoch root); earlier updates are `None`, since intermediate roots
    /// are not in the checkpoint logs.
    state: Option<Hash>,

    /// The inbox cursor before this update.
    prev_next_read_idx: u64,

    /// The inbox cursor after this update.
    next_read_idx: u64,

    /// The seqno after the update.
    seqno: Seqno,
}

impl SnarkAcctStateUpdate {
    pub fn new(
        account_id: AccountId,
        state: Option<Hash>,
        prev_next_read_idx: u64,
        next_read_idx: u64,
        seqno: Seqno,
    ) -> Self {
        Self {
            account_id,
            state,
            prev_next_read_idx,
            next_read_idx,
            seqno,
        }
    }

    /// Returns the account ID for this update.
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Returns the new inner state root. `None` for non-terminal checkpoint-sync
    /// updates (intermediate roots are unavailable).
    pub fn state(&self) -> Option<Hash> {
        self.state
    }

    /// Returns the inbox cursor before this update.
    pub fn prev_next_read_idx(&self) -> u64 {
        self.prev_next_read_idx
    }

    /// Returns the inbox cursor after this update.
    pub fn next_read_idx(&self) -> u64 {
        self.next_read_idx
    }

    /// Returns the seqno for this update.
    pub fn seqno(&self) -> Seqno {
        self.seqno
    }

    /// Sets the inner state root.
    pub fn set_state(&mut self, state: Option<Hash>) {
        self.state = state;
    }
}

// ============================================================================
// Predicate key update tracking
// ============================================================================

/// A tracked update to a snark account's predicate (update) verification key.
#[derive(Clone, Debug)]
pub struct PredicateKeyUpdate {
    /// The account whose predicate key was updated.
    account_id: AccountId,

    /// The new predicate key.
    new_vk: PredicateKey,
}

impl PredicateKeyUpdate {
    pub fn new(account_id: AccountId, new_vk: PredicateKey) -> Self {
        Self { account_id, new_vk }
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    pub fn new_vk(&self) -> &PredicateKey {
        &self.new_vk
    }
}

// ============================================================================
// L1 block record tracking
// ============================================================================

/// A tracked L1 block record write.
#[derive(Clone, Debug)]
pub struct L1BlockRecordWrite {
    /// The L1 block height associated with the record.
    pub height: L1Height,

    /// The L1 block record that was appended.
    pub record: L1BlockRecord,
}

// ============================================================================
// Account creation tracking
// ============================================================================

/// A tracked account-creation event.
#[derive(Clone, Debug)]
pub struct AccountCreatedWrite {
    /// The id of the newly created account.
    account_id: AccountId,
}

impl AccountCreatedWrite {
    pub fn new(account_id: AccountId) -> Self {
        Self { account_id }
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }
}

// ============================================================================
// Collected writes container
// ============================================================================

/// Collection of all tracked writes from the indexer layer.
///
/// This struct is extensible - add new `Vec` fields for future tracked operations.
#[derive(Clone, Debug, Default)]
pub struct IndexerWrites {
    created_accounts: Vec<AccountCreatedWrite>,
    inbox_messages: Vec<InboxMessageWrite>,
    l1_block_records: Vec<L1BlockRecordWrite>,
    snark_acct_state_updates: Vec<SnarkAcctStateUpdate>,
    predicate_key_updates: Vec<PredicateKeyUpdate>,
}

impl IndexerWrites {
    /// Creates a new empty collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an account-creation event.
    pub fn push_created_account(&mut self, write: AccountCreatedWrite) {
        self.created_accounts.push(write);
    }

    /// Records an inbox message write.
    pub fn push_inbox_message(&mut self, write: InboxMessageWrite) {
        self.inbox_messages.push(write);
    }

    /// Records an L1 block record write.
    pub fn push_l1_block_record(&mut self, write: L1BlockRecordWrite) {
        self.l1_block_records.push(write);
    }

    /// Records a snark state update.
    pub fn push_snark_acct_update(&mut self, update: SnarkAcctStateUpdate) {
        self.snark_acct_state_updates.push(update);
    }

    /// Replaces all tracked snark state updates with `updates`.
    ///
    /// This is required to collect the granular snark updates during checkpoint sync because diff
    /// only doesn't contain the granular updates, which has to come from OL logs in the checkpoint.
    pub fn set_snark_acct_state_updates(&mut self, updates: Vec<SnarkAcctStateUpdate>) {
        self.snark_acct_state_updates = updates;
    }

    /// Records a predicate key update.
    pub fn push_predicate_key_update(&mut self, update: PredicateKeyUpdate) {
        self.predicate_key_updates.push(update);
    }

    /// Returns all tracked account-creation events.
    pub fn created_accounts(&self) -> &[AccountCreatedWrite] {
        &self.created_accounts
    }

    /// Returns all tracked inbox message writes.
    pub fn inbox_messages(&self) -> &[InboxMessageWrite] {
        &self.inbox_messages
    }

    /// Returns all tracked L1 block record writes.
    pub fn l1_block_records(&self) -> &[L1BlockRecordWrite] {
        &self.l1_block_records
    }

    /// Returns all tracked snark state updates.
    pub fn snark_state_updates(&self) -> &[SnarkAcctStateUpdate] {
        &self.snark_acct_state_updates
    }

    /// Returns all tracked predicate key updates.
    pub fn predicate_key_updates(&self) -> &[PredicateKeyUpdate] {
        &self.predicate_key_updates
    }

    /// Returns true if no writes have been tracked.
    pub fn is_empty(&self) -> bool {
        self.created_accounts.is_empty()
            && self.inbox_messages.is_empty()
            && self.l1_block_records.is_empty()
            && self.snark_acct_state_updates.is_empty()
            && self.predicate_key_updates.is_empty()
    }

    /// Extends this collection with writes from another.
    pub fn extend(&mut self, other: IndexerWrites) {
        self.created_accounts.extend(other.created_accounts);
        self.inbox_messages.extend(other.inbox_messages);
        self.l1_block_records.extend(other.l1_block_records);
        self.snark_acct_state_updates
            .extend(other.snark_acct_state_updates);
        self.predicate_key_updates
            .extend(other.predicate_key_updates);
    }
}
