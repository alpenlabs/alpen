//! Test utilities for block assembly tests.

use std::sync::Arc;

use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload, tree_hash::TreeHash};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_identifiers::MmrId;
use strata_snark_acct_types::{AccumulatorClaim, MessageEntry};
use strata_storage::{NodeStorage, create_node_storage};
use threadpool::ThreadPool;

use crate::context::BlockAssemblyContext;

/// Creates a test account ID with the given seed byte.
pub(crate) fn test_account_id(id: u8) -> AccountId {
    let mut bytes = [0u8; 32];
    bytes[0] = id;
    AccountId::new(bytes)
}

/// Creates a test hash with all bytes set to the given seed.
pub(crate) fn test_hash(seed: u8) -> Hash {
    Hash::from([seed; 32])
}

/// Creates a test storage instance backed by an in-memory sled database.
pub(crate) fn create_test_storage() -> Arc<NodeStorage> {
    let pool = ThreadPool::new(1);
    let test_db = get_test_sled_backend();
    Arc::new(create_node_storage(test_db, pool).unwrap())
}

/// Creates a test message entry.
pub(crate) fn create_test_message(source_id: u8, epoch: u32, value_sats: u64) -> MessageEntry {
    let source = test_account_id(source_id);
    let payload = MsgPayload::new(BitcoinAmount::from_sat(value_sats), vec![1, 2, 3]);
    MessageEntry::new(source, epoch, payload)
}

/// Creates a minimal context for testing `AccumulatorProofGenerator`.
///
/// Uses unit types for mempool and state provider since
/// proof generation only requires storage access.
pub(crate) fn create_test_context(storage: Arc<NodeStorage>) -> BlockAssemblyContext<(), ()> {
    BlockAssemblyContext::new(storage, (), ())
}

// ===== Storage MMR Helpers =====

/// Tracks inbox MMR entries for a specific account in storage.
///
/// Use this to populate the storage MMR with messages, then create transactions
/// that reference those messages. Block assembly will generate proofs from storage.
pub(crate) struct StorageInboxMmr<'a> {
    storage: &'a NodeStorage,
    account_id: AccountId,
    entries: Vec<MessageEntry>,
    indices: Vec<u64>,
}

impl<'a> StorageInboxMmr<'a> {
    /// Creates a new tracker bound to storage for the given account.
    pub(crate) fn new(storage: &'a NodeStorage, account_id: AccountId) -> Self {
        Self {
            storage,
            account_id,
            entries: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Adds a message to the storage MMR and tracks it.
    pub(crate) fn add_message(&mut self, message: MessageEntry) -> u64 {
        let mmr_handle = self
            .storage
            .global_mmr()
            .as_ref()
            .get_handle(MmrId::SnarkMsgInbox(self.account_id));

        let hash = <MessageEntry as TreeHash>::tree_hash_root(&message);
        let idx = mmr_handle
            .append_leaf_blocking(hash.into_inner().into())
            .unwrap();

        self.entries.push(message);
        self.indices.push(idx);
        idx
    }

    /// Adds multiple messages and returns their indices.
    pub(crate) fn add_messages(
        &mut self,
        messages: impl IntoIterator<Item = MessageEntry>,
    ) -> Vec<u64> {
        messages
            .into_iter()
            .map(|msg| self.add_message(msg))
            .collect()
    }

    /// Returns the tracked message entries.
    pub(crate) fn entries(&self) -> &[MessageEntry] {
        &self.entries
    }
}

/// Tracks ASM MMR entries (L1 header hashes) in storage.
///
/// Use this to populate the storage MMR with L1 header hashes for claim validation tests.
pub(crate) struct StorageAsmMmr<'a> {
    storage: &'a NodeStorage,
    entries: Vec<Hash>,
    indices: Vec<u64>,
}

impl<'a> StorageAsmMmr<'a> {
    /// Creates a new tracker bound to storage.
    pub(crate) fn new(storage: &'a NodeStorage) -> Self {
        Self {
            storage,
            entries: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Adds a header hash to the storage MMR and tracks it.
    pub(crate) fn add_header(&mut self, hash: Hash) -> u64 {
        let mmr_handle = self.storage.global_mmr().as_ref().get_handle(MmrId::Asm);
        let idx = mmr_handle.append_leaf_blocking(hash).unwrap();
        self.entries.push(hash);
        self.indices.push(idx);
        idx
    }

    /// Adds multiple header hashes and returns their indices.
    pub(crate) fn add_headers(&mut self, hashes: impl IntoIterator<Item = Hash>) -> Vec<u64> {
        hashes.into_iter().map(|h| self.add_header(h)).collect()
    }

    /// Returns the tracked header hashes.
    pub(crate) fn hashes(&self) -> &[Hash] {
        &self.entries
    }

    /// Returns the MMR leaf indices.
    pub(crate) fn indices(&self) -> &[u64] {
        &self.indices
    }

    /// Returns all (index, hash) pairs suitable for creating L1 claims.
    pub(crate) fn claims(&self) -> Vec<AccumulatorClaim> {
        self.indices
            .iter()
            .zip(self.entries.iter())
            .map(|(&idx, &hash)| AccumulatorClaim::new(idx, hash))
            .collect()
    }
}
