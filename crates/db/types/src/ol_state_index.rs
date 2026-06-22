//! Types around the [`OLStateIndexingDatabase`] schema.
//!
//! Captures the indexing data persisted for later querying. Account-type-agnostic:
//! any account may produce any kind of indexing record.
//!
//! Records derive [`serde::Serialize`] / [`serde::Deserialize`] and are persisted
//! as CBOR. Fields whose native types lack serde derives (e.g. `MessageEntry`)
//! are stored in their raw SSZ byte form; callers convert at the boundaries.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strata_codec::Codec;
use strata_identifiers::{AccountId, Epoch, EpochCommitment, Hash, OLBlockCommitment};

use crate::DbResult;

/// Global epoch-level indexing facts. Mutable until epoch finalization.
///
/// `epoch_commitment` is set once at epoch finalization; `created_accounts`
/// grows incrementally as blocks in the epoch execute (block-sync) or is
/// populated in one shot (checkpoint-sync). Each entry pairs the account
/// with the block that created it (block-sync), or `None` when block
/// attribution is unavailable (checkpoint-sync). `None`-attributed entries
/// are immune to per-block rollback and only drop on full-epoch rollback.
///
/// `last_applied_block` is the high-water mark of block-sync apply for this
/// epoch. Reads as `None` for fresh epochs and checkpoint-sync rows. Used by
/// `apply_block_indexing` to reject duplicate / out-of-order applies before
/// any tree mutation occurs.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EpochIndexingData {
    epoch_commitment: Option<EpochCommitment>,
    created_accounts: Vec<AccountCreatedRecord>,
    last_applied_block: Option<OLBlockCommitment>,
}

impl EpochIndexingData {
    pub fn new(
        epoch_commitment: Option<EpochCommitment>,
        created_accounts: Vec<AccountCreatedRecord>,
        last_applied_block: Option<OLBlockCommitment>,
    ) -> Self {
        Self {
            epoch_commitment,
            created_accounts,
            last_applied_block,
        }
    }

    pub fn epoch_commitment(&self) -> Option<&EpochCommitment> {
        self.epoch_commitment.as_ref()
    }

    pub fn created_accounts(&self) -> &[AccountCreatedRecord] {
        &self.created_accounts
    }

    /// Iterates just the account ids of created accounts, dropping block attribution.
    pub fn created_account_ids(&self) -> impl Iterator<Item = AccountId> + '_ {
        self.created_accounts.iter().map(|r| r.account)
    }

    pub fn last_applied_block(&self) -> Option<&OLBlockCommitment> {
        self.last_applied_block.as_ref()
    }

    pub fn set_last_applied_block(&mut self, block: OLBlockCommitment) {
        self.last_applied_block = Some(block);
    }

    /// Resets the high-water mark to `None` if its current slot is strictly
    /// greater than `slot`. Used by `rollback_to_block` to ensure subsequent
    /// applies past the cutoff are accepted again.
    pub fn clear_last_applied_block_after_slot(&mut self, slot: u64) {
        if self.last_applied_block.is_some_and(|b| b.slot() > slot) {
            self.last_applied_block = None;
        }
    }

    pub fn set_epoch_commitment(&mut self, commitment: EpochCommitment) {
        self.epoch_commitment = Some(commitment);
    }

    pub fn push_created_account(&mut self, acct: AccountId, block: Option<OLBlockCommitment>) {
        self.created_accounts
            .push(AccountCreatedRecord::new(acct, block));
    }

    /// Removes entries whose attributed block has slot strictly greater than
    /// `slot`. Entries with `None` attribution (checkpoint-sync) are never
    /// matched. Returns the dropped account ids in insertion order.
    pub fn drop_created_after_slot(&mut self, slot: u64) -> Vec<AccountId> {
        let mut dropped = Vec::new();
        self.created_accounts.retain(|r| {
            let drop = r.block.is_some_and(|c| c.slot() > slot);
            if drop {
                dropped.push(r.account);
            }
            !drop
        });
        dropped
    }
}

/// A record of a created account. Expected to contain more data in the future.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountCreatedRecord {
    /// The account that was created.
    account: AccountId,
    /// Block at which the account was created.
    block: Option<OLBlockCommitment>,
}

impl AccountCreatedRecord {
    pub fn new(account: AccountId, block_commitment: Option<OLBlockCommitment>) -> Self {
        Self {
            account,
            block: block_commitment,
        }
    }

    pub fn new_account(account: AccountId) -> Self {
        Self {
            account,
            block: None,
        }
    }

    pub fn account(&self) -> AccountId {
        self.account
    }

    pub fn block(&self) -> Option<OLBlockCommitment> {
        self.block
    }
}

/// Metadata for an account update.
///
/// `block_commitment` is `None` on checkpoint-sync rows (no per-block
/// attribution available). `new_state_root` is always present.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountUpdateMeta {
    block_commitment: Option<OLBlockCommitment>,
    new_state_root: Hash,
}

impl AccountUpdateMeta {
    pub fn new(block_commitment: Option<OLBlockCommitment>, new_state_root: Hash) -> Self {
        Self {
            block_commitment,
            new_state_root,
        }
    }

    pub fn block_commitment(&self) -> Option<&OLBlockCommitment> {
        self.block_commitment.as_ref()
    }

    pub fn new_state_root(&self) -> Hash {
        self.new_state_root
    }
}

/// Single snark account state update.
///
/// Messages consumed by this update are the inbox entries in the range
/// `[self.prev_next_inbox_idx, self.next_inbox_idx)`. Callers fetch the
/// actual entries from the inbox MMR when needed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountUpdateRecord {
    update_meta: Option<AccountUpdateMeta>,
    seq_no: u64,
    prev_next_inbox_idx: u64,
    next_inbox_idx: u64,
    extra_data: Option<Vec<u8>>,
}

impl AccountUpdateRecord {
    pub fn new(
        update_meta: Option<AccountUpdateMeta>,
        seq_no: u64,
        prev_next_inbox_idx: u64,
        next_inbox_idx: u64,
        extra_data: Option<Vec<u8>>,
    ) -> Self {
        Self {
            update_meta,
            seq_no,
            prev_next_inbox_idx,
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

    /// Returns the operation seqno that produced this record.
    ///
    /// The record stores the post-update account seqno. Snark account updates
    /// publish the pre-update operation seqno, so this is `seq_no - 1`.
    pub fn orig_acct_seq_no(&self) -> Option<u64> {
        self.seq_no.checked_sub(1)
    }

    pub fn prev_next_inbox_idx(&self) -> u64 {
        self.prev_next_inbox_idx
    }

    pub fn next_inbox_idx(&self) -> u64 {
        self.next_inbox_idx
    }

    pub fn extra_data(&self) -> Option<&[u8]> {
        self.extra_data.as_deref()
    }
}

/// Key used for both account update and account inbox tables.
///
/// Keying by [`EpochCommitment`] is not viable: the commitment is unknown
/// during intermediate steps within the epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Codec)]
pub struct AccountEpochKey {
    epoch: Epoch,
    account_id: AccountId,
}

impl AccountEpochKey {
    pub fn new(epoch: Epoch, account_id: AccountId) -> Self {
        Self { epoch, account_id }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
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

/// Indexing data produced by a block-sync or checkpoint-sync run.
#[derive(Clone, Debug, Default)]
pub struct IndexingWrites {
    created_accounts: Vec<AccountId>,
    account_updates: BTreeMap<AccountId, Vec<AccountUpdateRecord>>,
    account_inbox: BTreeMap<AccountId, Vec<InboxMessageRecord>>,
}

impl IndexingWrites {
    pub fn new(
        created_accounts: Vec<AccountId>,
        account_updates: BTreeMap<AccountId, Vec<AccountUpdateRecord>>,
        account_inbox: BTreeMap<AccountId, Vec<InboxMessageRecord>>,
    ) -> Self {
        Self {
            created_accounts,
            account_updates,
            account_inbox,
        }
    }

    pub fn created_accounts(&self) -> &[AccountId] {
        &self.created_accounts
    }

    pub fn account_updates(&self) -> &BTreeMap<AccountId, Vec<AccountUpdateRecord>> {
        &self.account_updates
    }

    pub fn account_inbox(&self) -> &BTreeMap<AccountId, Vec<InboxMessageRecord>> {
        &self.account_inbox
    }
}

/// Database for OL state indexing data.
///
/// Two write paths reflect the two producer modes:
/// - [`apply_epoch_indexing`](Self::apply_epoch_indexing): single atomic write for an entire epoch.
///   Used by checkpoint-sync producers.
/// - [`apply_block_indexing`](Self::apply_block_indexing): incremental per-block write. Used by
///   block-sync producers.
///
/// Block-sync also calls [`set_epoch_commitment`](Self::set_epoch_commitment)
/// once at epoch finalization to stamp the commitment onto the existing common
/// row; checkpoint-sync includes the commitment in its single write.
///
/// Both paths target the same tables; atomicity granularity differs.
#[cfg_attr(
    feature = "proxies",
    strata_db_macros::gen_proxy(error = crate::DbError, tracing_component = "storage:ol_state_indexing")
)]
pub trait OLStateIndexingDatabase: Send + Sync + 'static {
    /// Atomically persists an epoch's indexing data in a single call.
    ///
    /// Writes the common record, per-account update entries, per-account
    /// inbox entries, and creation-epoch index entries for newly created
    /// accounts. The common record's `epoch_commitment` is set from
    /// `commitment`. All in one transaction.
    fn apply_epoch_indexing(
        &self,
        commitment: EpochCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()>;

    /// Atomically applies a single block's incremental indexing writes.
    ///
    /// Appends to existing per-(account, epoch) entries, updates the common
    /// row's `created_accounts`, and inserts creation-epoch index entries
    /// for any newly created accounts. Errors with
    /// [`DbError::BlockIndexingConflict`](crate::DbError::BlockIndexingConflict)
    /// when `block.slot()` does not strictly advance past the last applied
    /// block for `epoch`.
    fn apply_block_indexing(
        &self,
        epoch: Epoch,
        block: OLBlockCommitment,
        writes: IndexingWrites,
    ) -> DbResult<()>;

    /// Atomically rolls back all block-attributed writes in `epoch` whose
    /// block slot is strictly greater than `block.slot()`. Records and
    /// creators tagged with `block.slot()` itself are kept. Entries with
    /// `None` attribution (checkpoint-sync) are preserved; they only drop
    /// when the entire epoch is dropped via [`Self::rollback_to_epoch`].
    ///
    /// Idempotent. Does not clear `EpochIndexingData.epoch_commitment`.
    fn rollback_to_block(&self, epoch: Epoch, block: OLBlockCommitment) -> DbResult<()>;

    /// Atomically drops all indexing data for epochs strictly greater than
    /// `epoch`. The given `epoch` is preserved. Idempotent.
    fn rollback_to_epoch(&self, epoch: Epoch) -> DbResult<()>;

    /// Sets the epoch commitment on the existing common row.
    ///
    /// Called once by block-sync producers at epoch finalization. Errors if
    /// no common row exists for the epoch.
    fn set_epoch_commitment(&self, epoch: Epoch, commitment: EpochCommitment) -> DbResult<()>;

    /// Returns the common indexing data for the given epoch.
    fn get_epoch_indexing_data(&self, epoch: Epoch) -> DbResult<Option<EpochIndexingData>>;

    /// Returns the per-(account, epoch) update records.
    ///
    /// Returns `None` when the account had no indexed activity in the epoch.
    fn get_account_update_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<AccountUpdateRecord>>>;

    /// Returns the per-(account, epoch) inbox records.
    ///
    /// Returns `None` when no inbox writes were recorded for the account in the epoch.
    fn get_account_inbox_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<InboxMessageRecord>>>;

    /// Returns the epoch in which an account was created.
    fn get_account_creation_epoch(&self, acct: AccountId) -> DbResult<Option<Epoch>>;
}
