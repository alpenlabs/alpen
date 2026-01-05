//! Test utilities for block assembly tests.

use std::sync::Arc;

use proptest::{arbitrary, prelude::*, strategy::ValueTree, test_runner::TestRunner};
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload, tree_hash::TreeHash};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_identifiers::MmrId;
use strata_ledger_types::{AccountTypeState, IStateAccessor, NewAccountData};
use strata_ol_chain_types_new::{TransactionAttachment, test_utils as ol_test_utils};
use strata_ol_mempool::OLMempoolTransaction;
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefs, MessageEntry, ProofState, UpdateOperationData,
};
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
//
// These helpers write directly to `NodeStorage` so block assembly can read the
// MMRs it uses during proof generation. They intentionally avoid in-memory
// trackers to keep test setup aligned with production.

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

    /// Adds random header hashes using proptest.
    pub(crate) fn add_random_headers(&mut self, count: usize) -> Vec<u64> {
        let hashes = generate_header_hashes(count);
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

    /// Returns all claims as AccumulatorClaim objects.
    pub(crate) fn claims(&self) -> Vec<AccumulatorClaim> {
        self.indices
            .iter()
            .zip(self.entries.iter())
            .map(|(&idx, &hash)| AccumulatorClaim::new(idx, hash))
            .collect()
    }
}

// ===== Mempool Transaction Builder =====

/// Builder for creating OLMempoolTransaction for snark account updates.
///
/// Simplifies test setup by providing a fluent API for specifying only the fields
/// needed for each test case.
pub(crate) struct MempoolSnarkTxBuilder {
    account_id: AccountId,
    seq_no: u64,
    processed_messages: Vec<MessageEntry>,
    new_msg_idx: u64,
    l1_claims: Vec<AccumulatorClaim>,
}

impl MempoolSnarkTxBuilder {
    /// Creates a new builder for the given account.
    pub(crate) fn new(account_id: AccountId) -> Self {
        Self {
            account_id,
            seq_no: 0,
            processed_messages: Vec::new(),
            new_msg_idx: 0,
            l1_claims: Vec::new(),
        }
    }

    /// Sets the sequence number for this update.
    #[expect(dead_code, reason = "used by next commit")]
    pub(crate) fn with_seq_no(mut self, seq_no: u64) -> Self {
        self.seq_no = seq_no;
        self
    }

    /// Sets the processed inbox messages and updates new_msg_idx accordingly.
    pub(crate) fn with_processed_messages(mut self, messages: Vec<MessageEntry>) -> Self {
        self.new_msg_idx = messages.len() as u64;
        self.processed_messages = messages;
        self
    }

    /// Sets L1 header claims from AccumulatorClaim objects.
    pub(crate) fn with_l1_claims(mut self, claims: Vec<AccumulatorClaim>) -> Self {
        self.l1_claims = claims;
        self
    }

    /// Explicitly sets the new message index (for testing invalid indices).
    pub(crate) fn with_new_msg_idx(mut self, idx: u64) -> Self {
        self.new_msg_idx = idx;
        self
    }

    /// Builds the mempool transaction.
    pub(crate) fn build(self) -> OLMempoolTransaction {
        let mut runner = TestRunner::default();
        let attachment = TransactionAttachment::new(None, None);

        let full_payload = ol_test_utils::snark_account_update_tx_payload_strategy()
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let inner_state = full_payload
            .update_container
            .base_update
            .operation
            .new_proof_state()
            .inner_state();
        let new_proof_state = ProofState::new(inner_state, self.new_msg_idx);

        let claims: Vec<AccumulatorClaim> = self.l1_claims.into_iter().collect();
        let ledger_refs = if claims.is_empty() {
            LedgerRefs::new_empty()
        } else {
            LedgerRefs::new(claims)
        };

        let operation = UpdateOperationData::new(
            self.seq_no,
            new_proof_state,
            self.processed_messages,
            ledger_refs,
            full_payload
                .update_container
                .base_update
                .operation
                .outputs()
                .clone(),
            full_payload
                .update_container
                .base_update
                .operation
                .extra_data()
                .to_vec(),
        );

        let mut update = full_payload.update_container.base_update;
        update.operation = operation;

        OLMempoolTransaction::new_snark_account_update(self.account_id, update, attachment)
    }
}

pub(crate) fn add_snark_account_to_state(
    state: &mut OLState,
    account_id: AccountId,
    state_root_seed: u8,
    initial_balance: u64,
) {
    let snark_state =
        OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), test_hash(state_root_seed));
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(initial_balance),
        AccountTypeState::Snark(snark_state),
    );
    state.create_new_account(account_id, new_acct).unwrap();
}

/// Generate random MessageEntry objects using proptest.
pub(crate) fn generate_message_entries(
    count: usize,
    source_account: AccountId,
) -> Vec<MessageEntry> {
    let mut runner = TestRunner::default();
    (0..count)
        .map(|_| {
            let incl_epoch = (1u32..1000u32).new_tree(&mut runner).unwrap().current();
            let value_sats = (1u64..1000000u64).new_tree(&mut runner).unwrap().current();
            let data_len: usize = (0usize..32usize).new_tree(&mut runner).unwrap().current();
            let data: Vec<u8> = (0..data_len)
                .map(|_| {
                    arbitrary::any::<u8>()
                        .new_tree(&mut runner)
                        .unwrap()
                        .current()
                })
                .collect();

            let payload = MsgPayload::new(BitcoinAmount::from_sat(value_sats), data);
            MessageEntry::new(source_account, incl_epoch, payload)
        })
        .collect()
}

/// Generate random L1 header hashes using proptest.
pub(crate) fn generate_header_hashes(count: usize) -> Vec<Hash> {
    let mut runner = TestRunner::default();
    (0..count)
        .map(|_| {
            arbitrary::any::<[u8; 32]>()
                .new_tree(&mut runner)
                .unwrap()
                .current()
                .into()
        })
        .collect()
}
