//! Record types for the [`OLStateIndexingDatabase`] schema.
//!
//! Captures the indexing data persisted for later querying. Account-type-agnostic:
//! any account may produce any kind of indexing record.
//!
//! Records derive [`serde::Serialize`] / [`serde::Deserialize`] and are persisted
//! as CBOR. Fields whose native types lack serde derives (e.g. `MessageEntry`)
//! are stored in their raw SSZ byte form; callers convert at the boundaries.
//!
//! [`OLStateIndexingDatabase`]: crate::traits::OLStateIndexingDatabase

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strata_identifiers::{AccountId, Epoch, EpochCommitment, Hash, OLBlockCommitment};

/// Global epoch-level indexing facts. Mutable until epoch finalization.
///
/// `epoch_commitment` is set once at epoch finalization; `created_accounts`
/// grows incrementally as blocks in the epoch execute (block-sync) or is
/// populated in one shot (checkpoint-sync).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EpochIndexingData {
    epoch_commitment: Option<EpochCommitment>,
    created_accounts: Vec<AccountId>,
}

impl EpochIndexingData {
    pub fn new(
        epoch_commitment: Option<EpochCommitment>,
        created_accounts: Vec<AccountId>,
    ) -> Self {
        Self {
            epoch_commitment,
            created_accounts,
        }
    }

    pub fn epoch_commitment(&self) -> Option<&EpochCommitment> {
        self.epoch_commitment.as_ref()
    }

    pub fn created_accounts(&self) -> &[AccountId] {
        &self.created_accounts
    }

    pub fn set_epoch_commitment(&mut self, commitment: EpochCommitment) {
        self.epoch_commitment = Some(commitment);
    }

    pub fn push_created_account(&mut self, acct: AccountId) {
        self.created_accounts.push(acct);
    }
}

/// Block-sync metadata for an account update.
///
/// Present only when produced by a block-syncing node; checkpoint-sync
/// records leave this `None`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountUpdateMeta {
    block_commitment: OLBlockCommitment,
    final_state_root: Hash,
}

impl AccountUpdateMeta {
    pub fn new(block_commitment: OLBlockCommitment, final_state_root: Hash) -> Self {
        Self {
            block_commitment,
            final_state_root,
        }
    }

    pub fn block_commitment(&self) -> &OLBlockCommitment {
        &self.block_commitment
    }

    pub fn final_state_root(&self) -> Hash {
        self.final_state_root
    }
}

/// Single snark account state update.
///
/// `processed_messages` is the SSZ-encoded `MessageEntry`s consumed by this
/// update. Callers decode at the boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountUpdateRecord {
    update_meta: Option<AccountUpdateMeta>,
    seq_no: u64,
    processed_messages: Vec<Vec<u8>>,
    next_inbox_idx: u64,
    extra_data: Vec<u8>,
}

impl AccountUpdateRecord {
    pub fn new(
        update_meta: Option<AccountUpdateMeta>,
        seq_no: u64,
        processed_messages: Vec<Vec<u8>>,
        next_inbox_idx: u64,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            update_meta,
            seq_no,
            processed_messages,
            next_inbox_idx,
            extra_data,
        }
    }

    pub fn update_meta(&self) -> Option<&AccountUpdateMeta> {
        self.update_meta.as_ref()
    }

    pub fn seq_no(&self) -> u64 {
        self.seq_no
    }

    pub fn processed_messages(&self) -> &[Vec<u8>] {
        &self.processed_messages
    }

    pub fn next_inbox_idx(&self) -> u64 {
        self.next_inbox_idx
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}

/// Key used for both [`AccountUpdateEntry`] and [`AccountInboxEntry`].
///
/// Keying by [`EpochCommitment`] is not viable: the commitment is unknown
/// during intermediate steps within the epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct AccountEpochKey {
    pub epoch: Epoch,
    pub account_id: AccountId,
}

impl AccountEpochKey {
    pub fn new(epoch: Epoch, account_id: AccountId) -> Self {
        Self { epoch, account_id }
    }
}

/// Per-(account, epoch) list of update records.
///
/// Block-sync producers append records as blocks execute; checkpoint-sync
/// producers write the full list in one shot.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountUpdateEntry {
    records: Vec<AccountUpdateRecord>,
}

impl AccountUpdateEntry {
    pub fn new(records: Vec<AccountUpdateRecord>) -> Self {
        Self { records }
    }

    pub fn records(&self) -> &[AccountUpdateRecord] {
        &self.records
    }

    pub fn push(&mut self, record: AccountUpdateRecord) {
        self.records.push(record);
    }

    pub fn extend(&mut self, records: impl IntoIterator<Item = AccountUpdateRecord>) {
        self.records.extend(records);
    }
}

/// Inbox message append, optionally tagged with the block in which it was inserted.
///
/// `block_commitment` is `Some` for block-sync writes, `None` for checkpoint-sync.
/// `entry_bytes` is the SSZ encoding of the underlying `MessageEntry`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InboxMessageRecord {
    entry_bytes: Vec<u8>,
    block_commitment: Option<OLBlockCommitment>,
}

impl InboxMessageRecord {
    pub fn new(entry_bytes: Vec<u8>, block_commitment: Option<OLBlockCommitment>) -> Self {
        Self {
            entry_bytes,
            block_commitment,
        }
    }

    pub fn entry_bytes(&self) -> &[u8] {
        &self.entry_bytes
    }

    pub fn block_commitment(&self) -> Option<&OLBlockCommitment> {
        self.block_commitment.as_ref()
    }
}

/// Per-(account, epoch) list of inbox message writes.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountInboxEntry {
    records: Vec<InboxMessageRecord>,
}

impl AccountInboxEntry {
    pub fn new(records: Vec<InboxMessageRecord>) -> Self {
        Self { records }
    }

    pub fn records(&self) -> &[InboxMessageRecord] {
        &self.records
    }

    pub fn push(&mut self, record: InboxMessageRecord) {
        self.records.push(record);
    }

    pub fn extend(&mut self, records: impl IntoIterator<Item = InboxMessageRecord>) {
        self.records.extend(records);
    }
}

/// Per-block input payload for block-sync indexing writes.
///
/// One call to `apply_block_indexing` consumes one of these; the DB appends
/// to existing per-(account, epoch) rows and updates the common epoch row
/// with any newly created accounts.
#[derive(Clone, Debug, Default)]
pub struct BlockIndexingWrites {
    pub epoch: Epoch,
    pub block: OLBlockCommitment,
    pub created_accounts: Vec<AccountId>,
    pub account_updates: BTreeMap<AccountId, Vec<AccountUpdateRecord>>,
    pub account_inbox_writes: BTreeMap<AccountId, Vec<InboxMessageRecord>>,
}

impl BlockIndexingWrites {
    pub fn new(epoch: Epoch, block: OLBlockCommitment) -> Self {
        Self {
            epoch,
            block,
            created_accounts: Vec::new(),
            account_updates: BTreeMap::new(),
            account_inbox_writes: BTreeMap::new(),
        }
    }
}

/// Single-call payload for checkpoint-sync indexing writes.
///
/// Applied atomically by
/// [`OLStateIndexingDatabase::apply_epoch_indexing`](crate::traits::OLStateIndexingDatabase::apply_epoch_indexing).
#[derive(Clone, Debug)]
pub struct EpochIndexingWrites {
    pub epoch: Epoch,
    pub common: EpochIndexingData,
    pub account_updates: BTreeMap<AccountId, AccountUpdateEntry>,
    pub account_inbox: BTreeMap<AccountId, AccountInboxEntry>,
}

impl EpochIndexingWrites {
    pub fn new(epoch: Epoch, common: EpochIndexingData) -> Self {
        Self {
            epoch,
            common,
            account_updates: BTreeMap::new(),
            account_inbox: BTreeMap::new(),
        }
    }
}
