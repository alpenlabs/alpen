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
use strata_codec::Codec;
use strata_identifiers::{AccountId, Epoch, EpochCommitment, Hash, OLBlockCommitment};

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
/// Messages consumed by this update are the inbox entries in the range
/// `[prev_record.next_inbox_idx, self.next_inbox_idx)`. The first record
/// in an epoch uses the prior epoch's terminal `next_inbox_idx` as the
/// lower bound. Callers fetch the actual entries from the inbox MMR
/// when needed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountUpdateRecord {
    update_meta: Option<AccountUpdateMeta>,
    seq_no: u64,
    next_inbox_idx: u64,
    extra_data: Option<Vec<u8>>,
}

impl AccountUpdateRecord {
    pub fn new(
        update_meta: Option<AccountUpdateMeta>,
        seq_no: u64,
        next_inbox_idx: u64,
        extra_data: Option<Vec<u8>>,
    ) -> Self {
        Self {
            update_meta,
            seq_no,
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
