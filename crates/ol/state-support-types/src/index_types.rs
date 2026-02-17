//! Types for representing collected index data from state operations.
//!
//! This module contains types that capture operations performed on state
//! for later use by indexers. These are produced by the `IndexerState` layer.
// TODO make the field names here more consistent, which should also reflect in
// the spec and state accessor fn/arg names

use strata_acct_types::{AccountId, Hash};
use strata_asm_manifest_types::AsmManifest;
use strata_identifiers::L1Height;
use strata_snark_acct_types::{MessageEntry, Seqno};

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

/// A direct set via `set_proof_state_directly`.
#[derive(Clone, Debug)]
pub struct SAStateSetOp {
    /// The account whose state was updated.
    account_id: AccountId,

    /// The new inner state root.
    state: Hash,

    /// The next read index.
    next_read_idx: u64,

    /// The seqno after the update.
    seqno: Seqno,
}

impl SAStateSetOp {
    pub fn new(account_id: AccountId, state: Hash, next_read_idx: u64, seqno: Seqno) -> Self {
        Self {
            account_id,
            state,
            next_read_idx,
            seqno,
        }
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    pub fn state(&self) -> [u8; 32] {
        self.state.into()
    }

    pub fn next_read_idx(&self) -> u64 {
        self.next_read_idx
    }

    pub fn seqno(&self) -> Seqno {
        self.seqno
    }
}

/// An update via `update_inner_state` with extra data for DA.
#[derive(Clone, Debug)]
pub struct SAStateUpdateOp {
    /// The account whose state was updated.
    account_id: AccountId,

    /// The new inner state root.
    inner_state: Hash,

    /// The next read index.
    next_read_idx: u64,

    /// The seqno after the update.
    seqno: Seqno,

    /// The extra data associated with this update (for DA).
    extra_data: Vec<u8>,
}

impl SAStateUpdateOp {
    pub fn new(
        account_id: AccountId,
        inner_state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            account_id,
            inner_state,
            next_read_idx,
            seqno,
            extra_data,
        }
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    pub fn inner_state(&self) -> [u8; 32] {
        self.inner_state.into()
    }

    pub fn next_read_idx(&self) -> u64 {
        self.next_read_idx
    }

    pub fn seqno(&self) -> Seqno {
        self.seqno
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}

/// A tracked snark account state update.
///
/// This captures both `set_proof_state_directly` and `update_inner_state` calls.
#[derive(Clone, Debug)]
pub enum SnarkAcctStateUpdate {
    /// A direct set via `set_proof_state_directly`.
    DirectSet(SAStateSetOp),

    /// An update via `update_inner_state` with extra data for DA.
    Update(SAStateUpdateOp),
}

impl SnarkAcctStateUpdate {
    /// Returns the account ID for this update.
    pub fn account_id(&self) -> AccountId {
        match self {
            Self::DirectSet(s) => s.account_id,
            Self::Update(s) => s.account_id,
        }
    }

    /// Returns the state hash for this update.
    pub fn state(&self) -> Hash {
        match self {
            Self::DirectSet(s) => s.state,
            Self::Update(s) => s.inner_state,
        }
    }

    /// Returns the next read index for this update.
    pub fn next_read_idx(&self) -> u64 {
        match self {
            Self::DirectSet(s) => s.next_read_idx,
            Self::Update(s) => s.next_read_idx,
        }
    }

    /// Returns the seqno for this update.
    pub fn seqno(&self) -> Seqno {
        match self {
            Self::DirectSet(s) => s.seqno,
            Self::Update(s) => s.seqno,
        }
    }

    pub fn extra_data(&self) -> Option<&[u8]> {
        match self {
            Self::DirectSet(_) => None,
            Self::Update(u) => Some(u.extra_data()),
        }
    }
}

// ============================================================================
// Manifest tracking
// ============================================================================

/// A tracked manifest write.
#[derive(Clone, Debug)]
pub struct ManifestWrite {
    /// The L1 block height associated with the manifest.
    pub height: L1Height,

    /// The manifest that was appended.
    pub manifest: AsmManifest,
}

// ============================================================================
// Collected writes container
// ============================================================================

/// Collection of all tracked writes from the indexer layer.
///
/// This struct is extensible - add new `Vec` fields for future tracked operations.
#[derive(Clone, Debug, Default)]
pub struct IndexerWrites {
    inbox_messages: Vec<InboxMessageWrite>,
    manifests: Vec<ManifestWrite>,
    snark_acct_state_updates: Vec<SnarkAcctStateUpdate>,
}

impl IndexerWrites {
    /// Creates a new empty collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an inbox message write.
    pub fn push_inbox_message(&mut self, write: InboxMessageWrite) {
        self.inbox_messages.push(write);
    }

    /// Records a manifest write.
    pub fn push_manifest(&mut self, write: ManifestWrite) {
        self.manifests.push(write);
    }

    /// Records a snark state update.
    pub fn push_snark_acct_update(&mut self, update: SnarkAcctStateUpdate) {
        self.snark_acct_state_updates.push(update);
    }

    /// Returns all tracked inbox message writes.
    pub fn inbox_messages(&self) -> &[InboxMessageWrite] {
        &self.inbox_messages
    }

    /// Returns all tracked manifest writes.
    pub fn manifests(&self) -> &[ManifestWrite] {
        &self.manifests
    }

    /// Returns all tracked snark state updates.
    pub fn snark_state_updates(&self) -> &[SnarkAcctStateUpdate] {
        &self.snark_acct_state_updates
    }

    /// Returns true if no writes have been tracked.
    pub fn is_empty(&self) -> bool {
        self.inbox_messages.is_empty()
            && self.manifests.is_empty()
            && self.snark_acct_state_updates.is_empty()
    }

    /// Extends this collection with writes from another.
    pub fn extend(&mut self, other: IndexerWrites) {
        self.inbox_messages.extend(other.inbox_messages);
        self.manifests.extend(other.manifests);
        self.snark_acct_state_updates
            .extend(other.snark_acct_state_updates);
    }
}
