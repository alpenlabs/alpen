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

use borsh::{BorshDeserialize, BorshSerialize};
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
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
)]
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
