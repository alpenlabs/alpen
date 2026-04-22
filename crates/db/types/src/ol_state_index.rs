//! Record types for the [`OLStateIndexingDatabase`] schema.
//!
//! These types capture the epoch-granularity indexing data persisted for
//! later querying. They are account-type-agnostic: any account may produce
//! any kind of indexing record.
//!
//! Records derive [`serde::Serialize`] / [`serde::Deserialize`] and are
//! persisted as CBOR. Fields whose native types lack serde derives are stored
//! in their raw byte forms (e.g. `MessageEntry` as its SSZ encoding); callers
//! convert at the producer/consumer boundaries.
//!
//! [`OLStateIndexingDatabase`]: crate::traits::OLStateIndexingDatabase

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use strata_identifiers::{AccountId, Epoch, EpochCommitment, Hash};
use thiserror::Error;

/// Per-update record of a snark account state transition.
///
/// `extra_data.is_some()` corresponds to an `update_inner_state` call;
/// `extra_data.is_none()` corresponds to a `set_proof_state_directly` call.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnarkUpdateRecord {
    /// Sequence number after this update. Raw `u64`; callers convert to the
    /// `Seqno` newtype at the boundary.
    seqno: u64,

    /// Inner state root after this update.
    ///
    /// `None` when the producer (e.g. checkpoint sync) does not know the
    /// intermediate per-update state.
    new_inner_state: Option<Hash>,

    /// Inbox read frontier after this update.
    next_inbox_msg_idx: u64,

    /// Extra data associated with an update (for DA).
    ///
    /// `None` for direct-set updates.
    extra_data: Option<Vec<u8>>,
}

impl SnarkUpdateRecord {
    pub fn new(
        seqno: u64,
        new_inner_state: Option<Hash>,
        next_inbox_msg_idx: u64,
        extra_data: Option<Vec<u8>>,
    ) -> Self {
        Self {
            seqno,
            new_inner_state,
            next_inbox_msg_idx,
            extra_data,
        }
    }

    pub fn seqno(&self) -> u64 {
        self.seqno
    }

    pub fn new_inner_state(&self) -> Option<Hash> {
        self.new_inner_state
    }

    pub fn next_inbox_msg_idx(&self) -> u64 {
        self.next_inbox_msg_idx
    }

    pub fn extra_data(&self) -> Option<&[u8]> {
        self.extra_data.as_deref()
    }
}

/// Record of a single inbox message insertion.
///
/// `entry_bytes` holds the SSZ encoding of the underlying `MessageEntry`;
/// callers decode at the boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InboxMessageRecord {
    /// Position of this entry in the account's inbox MMR.
    mmr_index: u64,

    /// SSZ-encoded bytes of the message entry.
    entry_bytes: Vec<u8>,
}

impl InboxMessageRecord {
    pub fn new(mmr_index: u64, entry_bytes: Vec<u8>) -> Self {
        Self {
            mmr_index,
            entry_bytes,
        }
    }

    pub fn mmr_index(&self) -> u64 {
        self.mmr_index
    }

    pub fn entry_bytes(&self) -> &[u8] {
        &self.entry_bytes
    }
}

/// Per-account indexing record for a single epoch.
///
/// Only written when the account had indexing-relevant activity in the epoch.
/// Absence of a record means no activity.
///
/// # Invariant
///
/// When `snark_updates` is non-empty:
///   - Its last element's `next_inbox_msg_idx` equals `final_next_inbox_msg_idx`.
///   - If its last element's `new_inner_state` is `Some(s)`, then
///     `s == final_inner_state`.
///
/// Enforced by the constructors; fields are private and never mutated after
/// construction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountEpochRecord {
    snark_updates: Vec<SnarkUpdateRecord>,
    inbox_writes: Vec<InboxMessageRecord>,
    final_inner_state: Hash,
    final_next_inbox_msg_idx: u64,
}

impl AccountEpochRecord {
    /// Constructs a record from a non-empty list of per-update details.
    ///
    /// Derives the final state from the last update.
    ///
    /// Used by the full-sync producer.
    pub fn from_updates(
        snark_updates: Vec<SnarkUpdateRecord>,
        inbox_writes: Vec<InboxMessageRecord>,
    ) -> Result<Self, IndexingDataError> {
        let last = snark_updates
            .last()
            .ok_or(IndexingDataError::EmptySnarkUpdates)?;
        let final_inner_state = last
            .new_inner_state
            .ok_or(IndexingDataError::MissingFinalInnerState)?;
        let final_next_inbox_msg_idx = last.next_inbox_msg_idx;
        Ok(Self {
            snark_updates,
            inbox_writes,
            final_inner_state,
            final_next_inbox_msg_idx,
        })
    }

    /// Constructs a record from only end-of-epoch state.
    ///
    /// `snark_updates` is left empty. Used by the checkpoint-sync producer
    /// when per-update detail is unavailable.
    pub fn from_final_state(
        final_inner_state: Hash,
        final_next_inbox_msg_idx: u64,
        inbox_writes: Vec<InboxMessageRecord>,
    ) -> Self {
        Self {
            snark_updates: Vec::new(),
            inbox_writes,
            final_inner_state,
            final_next_inbox_msg_idx,
        }
    }

    pub fn snark_updates(&self) -> &[SnarkUpdateRecord] {
        &self.snark_updates
    }

    pub fn inbox_writes(&self) -> &[InboxMessageRecord] {
        &self.inbox_writes
    }

    pub fn final_inner_state(&self) -> Hash {
        self.final_inner_state
    }

    pub fn final_next_inbox_msg_idx(&self) -> u64 {
        self.final_next_inbox_msg_idx
    }
}

/// Epoch-level indexing facts not scoped to a single account.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommonEpochRecord {
    /// Accounts created during this epoch.
    accounts_created: Vec<AccountId>,
}

impl CommonEpochRecord {
    pub fn new(accounts_created: Vec<AccountId>) -> Self {
        Self { accounts_created }
    }

    pub fn accounts_created(&self) -> &[AccountId] {
        &self.accounts_created
    }
}

/// Single-write unit for the indexing database.
///
/// Applied atomically by
/// [`OLStateIndexingDatabase::apply_epoch_indexing`](crate::traits::OLStateIndexingDatabase::apply_epoch_indexing).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EpochIndexingData {
    epoch: Epoch,
    epoch_commitment: EpochCommitment,
    common: CommonEpochRecord,
    accounts: BTreeMap<AccountId, AccountEpochRecord>,
}

impl EpochIndexingData {
    pub fn new(
        epoch: Epoch,
        epoch_commitment: EpochCommitment,
        common: CommonEpochRecord,
        accounts: BTreeMap<AccountId, AccountEpochRecord>,
    ) -> Self {
        Self {
            epoch,
            epoch_commitment,
            common,
            accounts,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn epoch_commitment(&self) -> &EpochCommitment {
        &self.epoch_commitment
    }

    pub fn common(&self) -> &CommonEpochRecord {
        &self.common
    }

    pub fn accounts(&self) -> &BTreeMap<AccountId, AccountEpochRecord> {
        &self.accounts
    }
}

/// Per-block producer-side staging record, pending epoch fold.
///
/// Not on the [`OLStateIndexingDatabase`] trait: a full-node producer concern
/// only (checkpoint sync builds [`EpochIndexingData`] directly).
///
/// [`OLStateIndexingDatabase`]: crate::traits::OLStateIndexingDatabase
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PerBlockStagingRecord {
    /// Accounts created in this block.
    accounts_created: Vec<AccountId>,

    /// Per-account indexing data from this block, in intra-block execution order.
    accounts: Vec<(AccountId, AccountBlockIndexData)>,
}

impl PerBlockStagingRecord {
    pub fn new(
        accounts_created: Vec<AccountId>,
        accounts: Vec<(AccountId, AccountBlockIndexData)>,
    ) -> Self {
        Self {
            accounts_created,
            accounts,
        }
    }

    pub fn accounts_created(&self) -> &[AccountId] {
        &self.accounts_created
    }

    pub fn accounts(&self) -> &[(AccountId, AccountBlockIndexData)] {
        &self.accounts
    }
}

/// One account's indexing data from within a single block.
///
/// Invariant: when `snark_updates` is non-empty, its last element's
/// `next_inbox_msg_idx` equals `final_next_inbox_msg_idx`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountBlockIndexData {
    snark_updates: Vec<SnarkUpdateRecord>,
    inbox_writes: Vec<InboxMessageRecord>,
    /// End-of-block inbox read frontier. Present even when there are no snark
    /// updates (inbox-only block), in which case it equals the pre-block frontier.
    final_next_inbox_msg_idx: u64,
}

impl AccountBlockIndexData {
    /// Builds from a non-empty update list; derives the frontier from the last update.
    pub fn from_updates(
        snark_updates: Vec<SnarkUpdateRecord>,
        inbox_writes: Vec<InboxMessageRecord>,
    ) -> Result<Self, IndexingDataError> {
        let last = snark_updates
            .last()
            .ok_or(IndexingDataError::EmptySnarkUpdates)?;
        let final_next_inbox_msg_idx = last.next_inbox_msg_idx;
        Ok(Self {
            snark_updates,
            inbox_writes,
            final_next_inbox_msg_idx,
        })
    }

    /// Builds for a block with no snark updates; frontier must be supplied.
    pub fn without_updates(
        inbox_writes: Vec<InboxMessageRecord>,
        final_next_inbox_msg_idx: u64,
    ) -> Self {
        Self {
            snark_updates: Vec::new(),
            inbox_writes,
            final_next_inbox_msg_idx,
        }
    }

    pub fn snark_updates(&self) -> &[SnarkUpdateRecord] {
        &self.snark_updates
    }

    pub fn inbox_writes(&self) -> &[InboxMessageRecord] {
        &self.inbox_writes
    }

    pub fn final_next_inbox_msg_idx(&self) -> u64 {
        self.final_next_inbox_msg_idx
    }
}

/// Errors returned while constructing indexing records.
#[derive(Debug, Error)]
pub enum IndexingDataError {
    /// `from_updates` called with no updates; cannot derive final state.
    #[error("snark update list is empty; cannot derive final state")]
    EmptySnarkUpdates,

    /// Last update is missing `new_inner_state`; cannot derive final state.
    #[error("last snark update has no inner state; cannot derive final state")]
    MissingFinalInnerState,
}
