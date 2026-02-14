//! Test utilities for block assembly tests.

use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use bitcoin::locktime::absolute::Height;
use proptest::{arbitrary, prelude::*, strategy::ValueTree, test_runner::TestRunner};
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload, tree_hash::TreeHash};
use strata_asm_common::{AnchorState, AsmHistoryAccumulatorState, ChainViewState};
use strata_asm_manifest_types::AsmManifest;
use strata_asm_types::HeaderVerificationState;
use strata_config::SequencerConfig;
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_types::errors::DbError;
use strata_identifiers::{
    Buf32, Buf64, L1BlockCommitment, L1BlockId, MmrId, OLBlockCommitment, OLBlockId, OLTxId,
    WtxidsRoot, test_utils::ol_block_commitment_strategy,
};
use strata_ledger_types::{
    AccountTypeState, IAccountStateMut, ISnarkAccountStateMut, IStateAccessor, NewAccountData,
};
use strata_ol_chain_types_new::{
    OLBlock, OLBlockBody, OLTxSegment, SignedOLBlockHeader, TransactionAttachment,
    test_utils as ol_test_utils,
};
use strata_ol_mempool::{MempoolTxInvalidReason, OLMempoolTransaction};
use strata_ol_state_types::{OLSnarkAccountState, OLState, StateProvider};
use strata_ol_stf::{BlockComponents, BlockContext, BlockInfo, construct_block};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefs, MessageEntry, OutputMessage, ProofState, UpdateOperationData,
    UpdateOutputs,
};
use strata_state::asm_state::AsmState;
use strata_storage::{NodeStorage, OLStateManager, create_node_storage};
use threadpool::ThreadPool;

use crate::{
    BlockAssemblyResult, FixedSlotSealing, MempoolProvider,
    context::BlockAssemblyContext,
    types::{BlockGenerationConfig, FullBlockTemplate},
};

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

/// Mock mempool provider for tests that stores transactions in memory.
pub(crate) struct MockMempoolProvider {
    transactions: Mutex<Vec<(OLTxId, OLMempoolTransaction)>>,
}

impl MockMempoolProvider {
    /// Create a new empty mock mempool provider.
    pub(crate) fn new() -> Self {
        Self {
            transactions: Mutex::new(Vec::new()),
        }
    }

    /// Add a transaction to the mock mempool.
    pub(crate) fn add_transaction(&self, txid: OLTxId, tx: OLMempoolTransaction) {
        self.transactions.lock().unwrap().push((txid, tx));
    }
}

#[async_trait]
impl MempoolProvider for MockMempoolProvider {
    async fn get_transactions(
        &self,
        limit: usize,
    ) -> BlockAssemblyResult<Vec<(OLTxId, OLMempoolTransaction)>> {
        let txs = self.transactions.lock().unwrap();
        Ok(txs.iter().take(limit).cloned().collect())
    }

    async fn report_invalid_transactions(
        &self,
        txs: &[(OLTxId, MempoolTxInvalidReason)],
    ) -> BlockAssemblyResult<()> {
        let mut stored = self.transactions.lock().unwrap();
        for (txid, _reason) in txs {
            stored.retain(|(id, _)| id != txid);
        }
        Ok(())
    }
}

#[async_trait]
impl MempoolProvider for Arc<MockMempoolProvider> {
    async fn get_transactions(
        &self,
        limit: usize,
    ) -> BlockAssemblyResult<Vec<(OLTxId, OLMempoolTransaction)>> {
        MempoolProvider::get_transactions(self.as_ref(), limit).await
    }

    async fn report_invalid_transactions(
        &self,
        txs: &[(OLTxId, MempoolTxInvalidReason)],
    ) -> BlockAssemblyResult<()> {
        MempoolProvider::report_invalid_transactions(self.as_ref(), txs).await
    }
}

pub(crate) struct StateProviderHandle(Arc<OLStateManager>);

impl StateProvider for StateProviderHandle {
    type State = OLState;
    type Error = DbError;

    fn get_state_for_tip_async(
        &self,
        tip: OLBlockCommitment,
    ) -> impl Future<Output = Result<Option<Arc<Self::State>>, Self::Error>> + Send {
        self.0.get_state_for_tip_async(tip)
    }

    fn get_state_for_tip_blocking(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<Arc<Self::State>>, Self::Error> {
        self.0.get_state_for_tip_blocking(tip)
    }
}

/// Concrete block assembly context for tests using mock implementations.
pub(crate) type BlockAssemblyContextImpl =
    BlockAssemblyContext<Arc<MockMempoolProvider>, StateProviderHandle>;

/// Number of slots per epoch used in tests.
pub(crate) const TEST_SLOTS_PER_EPOCH: u64 = 10;

/// TTL for block templates in tests. Matches DEFAULT_BLOCK_TEMPLATE_TTL_SECS from config crate.
pub(crate) const TEST_BLOCK_TEMPLATE_TTL: Duration = Duration::from_secs(60);

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
    outputs: Vec<(AccountId, u64)>,
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
            outputs: Vec::new(),
        }
    }

    /// Sets the sequence number for this update.
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

    /// Sets output messages (balance transfers to other accounts).
    pub(crate) fn with_outputs(mut self, outputs: Vec<(AccountId, u64)>) -> Self {
        self.outputs = outputs;
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

        let claims: Vec<AccumulatorClaim> = self.l1_claims.clone().into_iter().collect();
        let ledger_refs = if claims.is_empty() {
            LedgerRefs::new_empty()
        } else {
            LedgerRefs::new(self.l1_claims)
        };

        // Build outputs: empty by default, or explicit if with_outputs() was called
        let outputs = if self.outputs.is_empty() {
            UpdateOutputs::new_empty()
        } else {
            let output_messages: Vec<OutputMessage> = self
                .outputs
                .into_iter()
                .map(|(dest, value_sats)| {
                    let payload = MsgPayload::new(BitcoinAmount::from_sat(value_sats), vec![]);
                    OutputMessage::new(dest, payload)
                })
                .collect();
            UpdateOutputs::new(vec![], output_messages)
        };

        let operation = UpdateOperationData::new(
            self.seq_no,
            new_proof_state,
            self.processed_messages,
            ledger_refs,
            outputs,
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

/// Inserts inbox messages into a snark account's state MMR.
pub(crate) fn insert_inbox_messages_into_state(
    state: &mut OLState,
    account_id: AccountId,
    messages: &[MessageEntry],
) {
    for message in messages {
        state
            .update_account(account_id, |acct| {
                let snark_state = acct.as_snark_account_mut().expect("expected snark account");
                snark_state
                    .insert_inbox_message(message.clone())
                    .expect("insert inbox message");
            })
            .expect("update account");
    }
}

/// Inserts inbox messages into the stored OL state at `commitment`.
pub(crate) async fn insert_inbox_messages_into_storage_state(
    storage: &NodeStorage,
    commitment: OLBlockCommitment,
    account_id: AccountId,
    messages: &[MessageEntry],
) {
    let state = storage
        .ol_state()
        .get_toplevel_ol_state_async(commitment)
        .await
        .expect("fetch stored state")
        .expect("stored state missing");
    let mut state = (*state).clone();

    insert_inbox_messages_into_state(&mut state, account_id, messages);

    storage
        .ol_state()
        .put_toplevel_ol_state_async(commitment, state)
        .await
        .expect("store updated state");
}

/// Create test parent header by executing genesis block.
pub(crate) fn create_test_parent_header() -> strata_ol_chain_types_new::OLBlockHeader {
    let mut runner = TestRunner::default();
    let timestamp = (1000000u64..2000000u64)
        .new_tree(&mut runner)
        .unwrap()
        .current();

    let genesis_info = BlockInfo::new_genesis(timestamp);
    let mut temp_state = OLState::new_genesis();
    let genesis_context = BlockContext::new(&genesis_info, None);
    let genesis_components = BlockComponents::new_empty();
    let genesis_output =
        construct_block(&mut temp_state, genesis_context, genesis_components).unwrap();
    genesis_output.completed_block().header().clone()
}

/// Creates a random [`FullBlockTemplate`] using proptest strategies.
///
/// Each call produces a distinct template (random header fields).
pub(crate) fn create_test_template() -> FullBlockTemplate {
    let mut runner = TestRunner::default();
    let header = ol_test_utils::ol_block_header_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();
    let body = ol_test_utils::ol_block_body_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();
    FullBlockTemplate::new(header, body)
}

/// Creates a random [`FullBlockTemplate`] with a specific parent block ID.
///
/// Useful for testing cache eviction where multiple templates share the same parent.
pub(crate) fn create_test_template_with_parent(parent: OLBlockId) -> FullBlockTemplate {
    let mut runner = TestRunner::default();
    let mut header = ol_test_utils::ol_block_header_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();
    header.parent_blkid = parent;
    let body = ol_test_utils::ol_block_body_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();
    FullBlockTemplate::new(header, body)
}

/// Creates a random [`BlockGenerationConfig`] using proptest strategies.
pub(crate) fn create_test_block_generation_config() -> BlockGenerationConfig {
    let mut runner = TestRunner::default();
    let commitment = ol_block_commitment_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();
    BlockGenerationConfig::new(commitment)
}

/// Create test storage instance.
pub(crate) fn create_test_storage() -> Arc<NodeStorage> {
    let pool = ThreadPool::new(1);
    let test_db = get_test_sled_backend();
    Arc::new(create_node_storage(test_db, pool).unwrap())
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

// ===== Test Environment Builder (Commit 2) =====

/// Setup ASM state with L1 manifests in storage.
///
/// Creates and stores ASM manifests for L1 blocks from height `start` to `end` (inclusive),
/// and stores an ASM state at the highest L1 block.
///
/// Returns the L1BlockCommitment for the highest block.
pub(crate) async fn setup_asm_state_with_l1_manifests(
    storage: &NodeStorage,
    start: u64,
    end: u64,
) -> L1BlockCommitment {
    // Create and store ASM manifests
    let mut last_blkid = L1BlockId::from(Buf32::zero());
    for height in start..=end {
        // Generate deterministic but unique block ID for each height
        let mut block_bytes = [0u8; 32];
        block_bytes[0] = height as u8;
        block_bytes[1] = (height >> 8) as u8;
        last_blkid = L1BlockId::from(Buf32::from(block_bytes));

        let manifest = AsmManifest::new(
            height,
            last_blkid,
            WtxidsRoot::from(Buf32::from([0u8; 32])),
            vec![],
        );

        storage
            .l1()
            .put_block_data_async(manifest.clone())
            .await
            .expect("Failed to store L1 manifest");
        storage
            .l1()
            .extend_canonical_chain_async(manifest.blkid(), height)
            .await
            .expect("Failed to extend L1 canonical chain");
    }

    // Store ASM state at the highest L1 block
    let height = Height::from_consensus(end as u32).expect("Invalid height");
    let l1_commitment = L1BlockCommitment::new(height, last_blkid);

    // Create minimal ASM state for testing
    let pow_state = HeaderVerificationState::default();
    let history_accumulator = AsmHistoryAccumulatorState::new(0);
    let chain_view = ChainViewState {
        pow_state,
        history_accumulator,
    };
    let anchor_state = AnchorState {
        chain_view,
        sections: vec![],
    };
    let asm_state = AsmState::new(anchor_state, vec![]);

    storage
        .asm()
        .put_state(l1_commitment, asm_state)
        .expect("Failed to store ASM state");

    l1_commitment
}

/// Default balance for test accounts (100 billion sats).
pub(crate) const DEFAULT_ACCOUNT_BALANCE: u64 = 100_000_000_000;

/// Info about a manifest in the MMR - links index and hash together.
pub(crate) struct ManifestInfo {
    pub index: u64,
    pub hash: Hash,
}

/// Output from TestEnvBuilder - all fields public for direct access.
pub(crate) struct TestEnv {
    pub storage: Arc<NodeStorage>,
    pub parent_commitment: OLBlockCommitment,
    pub sequencer_config: SequencerConfig,
    pub epoch_sealing_policy: FixedSlotSealing,
    pub manifests: Vec<ManifestInfo>,
}

/// Builder for block assembly test environments.
#[derive(Default)]
pub(crate) struct TestEnvBuilder {
    parent_slot: Option<u64>,
    asm_manifest_heights: Vec<u64>,
    claim_manifest_count: Option<usize>,
    accounts: Vec<(AccountId, u64)>,
}

impl TestEnvBuilder {
    /// Creates a new builder with default values.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Sets the parent slot for the test environment.
    /// If not set, returns null commitment (for genesis testing).
    pub(crate) fn with_parent_slot(mut self, slot: u64) -> Self {
        self.parent_slot = Some(slot);
        self
    }

    /// Adds a snark account with the specified balance.
    pub(crate) fn with_account(mut self, id: AccountId, balance: u64) -> Self {
        self.accounts.push((id, balance));
        self
    }

    /// Stores L1 manifests in ASM storage for block's L1 update fetching.
    /// Used by tests that build terminal blocks with L1 manifests.
    pub(crate) fn with_asm_manifests(mut self, heights: &[u64]) -> Self {
        self.asm_manifest_heights = heights.to_vec();
        self
    }

    /// Sets up manifests in BOTH storage MMR AND state MMR for claim testing.
    /// The manifests field in TestEnv will be populated with ManifestInfo.
    pub(crate) fn with_claim_manifests(mut self, count: usize) -> Self {
        self.claim_manifest_count = Some(count);
        self
    }

    /// Builds the test environment.
    pub(crate) async fn build(self) -> TestEnv {
        let storage = create_test_storage();

        // Setup ASM state with L1 manifests if heights provided
        if let (Some(&min_height), Some(&max_height)) = (
            self.asm_manifest_heights.iter().min(),
            self.asm_manifest_heights.iter().max(),
        ) {
            setup_asm_state_with_l1_manifests(&storage, min_height, max_height).await;
        }

        // Create genesis state
        let mut state = OLState::new_genesis();

        // Add snark accounts
        for (i, (account_id, balance)) in self.accounts.iter().enumerate() {
            add_snark_account_to_state(&mut state, *account_id, i as u8 + 1, *balance);
        }

        // Setup claim manifests if requested (populates both state and storage MMRs)
        let manifests = if let Some(count) = self.claim_manifest_count {
            let test_manifests = create_deterministic_manifests(count);
            let (hashes, indices) =
                setup_manifests_in_state_and_storage(&storage, &mut state, test_manifests.clone());

            test_manifests
                .iter()
                .enumerate()
                .map(|(i, _)| ManifestInfo {
                    index: indices[i],
                    hash: hashes[i],
                })
                .collect()
        } else {
            vec![]
        };

        let parent_commitment = if let Some(slot) = self.parent_slot {
            let temp_header = create_test_parent_header();
            let temp_body = OLBlockBody::new_common(
                OLTxSegment::new(vec![]).expect("Failed to create tx segment"),
            );

            let (parent_state, parent_header, parent_block_body) = if slot == 0 {
                // Slot 0 is genesis - create terminal block
                let block_info = BlockInfo::new_genesis(1000000);

                // Create genesis manifest at height 0
                let genesis_manifest = AsmManifest::new(
                    0,
                    L1BlockId::from(Buf32::zero()),
                    WtxidsRoot::from(Buf32::zero()),
                    vec![],
                );
                let components = BlockComponents::new_manifests(vec![genesis_manifest]);

                let block_context = BlockContext::new(&block_info, None);
                let construct_output = construct_block(&mut state, block_context, components)
                    .expect("Genesis block execution should succeed");

                let completed_block = construct_output.completed_block();
                let header = completed_block.header().clone();
                let body = completed_block.body().clone();

                (state, header, body)
            } else {
                (state, temp_header, temp_body)
            };

            let commitment =
                OLBlockCommitment::new(parent_header.slot(), parent_header.compute_blkid());
            let parent_signed_header =
                SignedOLBlockHeader::new(parent_header.clone(), Buf64::zero());
            let parent_block = OLBlock::new(parent_signed_header, parent_block_body);

            storage
                .ol_state()
                .put_toplevel_ol_state_async(commitment, parent_state)
                .await
                .expect("Failed to store parent OL state");

            storage
                .ol_block()
                .put_block_data_async(parent_block)
                .await
                .expect("Failed to store parent block");

            commitment
        } else {
            // No parent slot - return null commitment for genesis testing
            let null_commitment = OLBlockCommitment::null();
            storage
                .ol_state()
                .put_toplevel_ol_state_async(null_commitment, state)
                .await
                .expect("Failed to store genesis OL state at null commitment");
            null_commitment
        };

        let sequencer_config = SequencerConfig::default();

        let epoch_sealing_policy = FixedSlotSealing::new(TEST_SLOTS_PER_EPOCH);

        TestEnv {
            storage,
            parent_commitment,
            sequencer_config,
            epoch_sealing_policy,
            manifests,
        }
    }
}

/// Create deterministic test manifests with unique block IDs.
///
/// Returns manifests that can be used to populate both state and storage MMRs.
fn create_deterministic_manifests(count: usize) -> Vec<AsmManifest> {
    (0..count)
        .map(|i| {
            let mut blkid_bytes = [0u8; 32];
            blkid_bytes[0] = (i + 1) as u8; // Unique block ID for each manifest
            AsmManifest::new(
                (i + 1) as u64, // height
                L1BlockId::from(Buf32::from(blkid_bytes)),
                WtxidsRoot::from(Buf32::zero()),
                vec![],
            )
        })
        .collect()
}

/// Setup manifests in both storage MMR and state's manifest MMR.
///
/// This ensures consistency between proof generation (uses storage MMR) and
/// verification (uses state's manifest MMR).
///
/// Returns the manifest hashes and their leaf indices.
fn setup_manifests_in_state_and_storage(
    storage: &NodeStorage,
    state: &mut OLState,
    manifests: Vec<AsmManifest>,
) -> (Vec<Hash>, Vec<u64>) {
    let mmr_handle = storage.global_mmr().as_ref().get_handle(MmrId::Asm);

    let mut hashes = Vec::with_capacity(manifests.len());
    let mut indices = Vec::with_capacity(manifests.len());

    for manifest in manifests {
        // Compute manifest hash
        let manifest_hash: Hash = manifest.compute_hash().into();
        hashes.push(manifest_hash);

        // Add to storage MMR (for proof generation)
        let leaf_idx = mmr_handle.append_leaf_blocking(manifest_hash).unwrap();
        indices.push(leaf_idx);

        // Add to state's manifest MMR (for verification)
        let height = manifest.height() as u32;
        state.append_manifest(height, manifest);
    }

    (hashes, indices)
}

/// Create test BlockAssemblyContext with mock providers.
///
/// Returns the context. Use `ctx.mempool_provider()` to add transactions to the mock mempool.
pub(crate) fn create_test_block_assembly_context(
    storage: Arc<NodeStorage>,
) -> (BlockAssemblyContextImpl, Arc<MockMempoolProvider>) {
    let mempool_provider = Arc::new(MockMempoolProvider::new());
    let state_provider = StateProviderHandle(storage.ol_state().clone());
    let ctx = BlockAssemblyContext::new(storage, mempool_provider.clone(), state_provider);
    (ctx, mempool_provider)
}
