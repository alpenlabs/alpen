//! Test utilities for block assembly tests.

use std::{
    future::Future,
    iter,
    ops::RangeInclusive,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use bitcoin::Network;
use proptest::{arbitrary, prelude::*, strategy::ValueTree, test_runner::TestRunner};
use strata_acct_types::{
    AccountId, AccountSerial, AccumulatorClaim, BitcoinAmount, Hash, MessageEntry, MsgPayload,
    tree_hash::TreeHash,
};
use strata_asm_common::{
    AnchorState, AsmHistoryAccumulatorState, ChainViewState, HeaderVerificationState,
};
use strata_asm_manifest_types::AsmManifest;
use strata_btc_verification::L1Anchor;
use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_config::SequencerConfig;
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_types::{DbError, MmrId};
use strata_identifiers::{
    Buf32, Buf64, L1BlockCommitment, L1BlockId, L1Height, OLBlockCommitment, OLBlockId, OLTxId,
    WtxidsRoot,
};
use strata_l1_txfmt::MagicBytes;
use strata_ledger_types::*;
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_chain_types_new::{
    ClaimList, OLBlock, OLBlockBody, OLLog, OLTransaction, OLTransactionData, OLTxSegment,
    ProofSatisfierList, SauTxLedgerRefs, SauTxOperationData, SauTxPayload, SauTxProofState,
    SauTxUpdateData, SignedOLBlockHeader, SimpleWithdrawalIntentLogData, TransactionPayload,
    TxProofs, test_utils as ol_test_utils,
};
use strata_ol_mempool::{MempoolTxInvalidReason, OLMempoolError};
use strata_ol_msg_types::{DEFAULT_OPERATOR_FEE, WITHDRAWAL_MSG_TYPE_ID, WithdrawalMsgData};
use strata_ol_params::OLParams;
use strata_ol_state_provider::{OLStateManagerProviderImpl, StateProvider};
use strata_ol_state_support_types::{EpochDaAccumulator, MemoryStateBaseLayer};
use strata_ol_state_types::OLState;
use strata_ol_stf::{
    BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL, BlockComponents, BlockContext, BlockInfo,
    construct_block as stf_construct_block,
};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::*;
use strata_state::asm_state::AsmState;
use strata_storage::{NodeStorage, create_node_storage};
use threadpool::ThreadPool;

/// Creates a genesis OLState using minimal empty parameters.
pub(crate) fn create_test_genesis_state() -> MemoryStateBaseLayer {
    let params = OLParams::new_empty(L1BlockCommitment::default());
    let state = OLState::from_genesis_params(&params).expect("valid params");
    MemoryStateBaseLayer::new(state)
}

use crate::{
    BlockAssemblyResult, FixedSlotSealing, MempoolProvider,
    block_assembly::{
        ConstructBlockOutput, calculate_block_slot_and_epoch, construct_block,
        generate_block_template_inner,
    },
    context::{BlockAssemblyAnchorContext, BlockAssemblyContext},
    da_tracker::AccumulatedDaData,
    types::{BlockGenerationConfig, BlockTemplateResult, FullBlockTemplate},
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

// ===== Post-State Query Helpers =====
//
// Keep account/snark assertions concise in tests that inspect post-state.

/// Returns account balance from post-state.
pub(crate) fn account_balance(state: &impl IStateAccessor, account_id: AccountId) -> BitcoinAmount {
    state
        .get_account_state(account_id)
        .expect("lookup account")
        .expect("account exists")
        .balance()
}

/// Returns snark account state from post-state.
pub(crate) fn snark_account_state<S: IStateAccessor>(
    state: &S,
    account_id: AccountId,
) -> &<S::AccountState as IAccountState>::SnarkAccountState {
    state
        .get_account_state(account_id)
        .expect("lookup account")
        .expect("account exists")
        .as_snark_account()
        .expect("account should be snark")
}

/// Returns snark account sequence number from post-state.
pub(crate) fn snark_account_seqno(state: &impl IStateAccessor, account_id: AccountId) -> u64 {
    *snark_account_state(state, account_id).seqno().inner()
}

/// Returns snark account next inbox message index from post-state.
pub(crate) fn snark_account_next_inbox_msg_idx(
    state: &impl IStateAccessor,
    account_id: AccountId,
) -> u64 {
    snark_account_state(state, account_id).next_inbox_msg_idx()
}

/// Returns snark account inbox MMR entry count from post-state.
pub(crate) fn snark_account_inbox_len(state: &impl IStateAccessor, account_id: AccountId) -> u64 {
    snark_account_state(state, account_id)
        .inbox_mmr()
        .num_entries()
}

/// Creates a test message entry.
pub(crate) fn create_test_message(source_id: u8, epoch: u32, value_sats: u64) -> MessageEntry {
    let source = test_account_id(source_id);
    let mut runner = TestRunner::default();
    let sampled_message = ol_test_utils::message_entry_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();
    let payload_bytes = sampled_message.payload().data().to_vec();
    let payload = MsgPayload::new(BitcoinAmount::from_sat(value_sats), payload_bytes);
    MessageEntry::new(source, epoch, payload)
}

/// Creates a minimal context for testing `AccumulatorProofGenerator`.
///
/// Uses unit types for mempool and state provider since
/// proof generation only requires storage access.
pub(crate) fn create_test_context(storage: Arc<NodeStorage>) -> BlockAssemblyContext<(), ()> {
    BlockAssemblyContext::new(storage, (), (), 0)
}

/// Mock mempool provider for tests that stores transactions in memory.
pub(crate) struct MockMempoolProvider {
    transactions: Mutex<Vec<(OLTxId, OLTransaction)>>,
    report_call_count: AtomicUsize,
    last_reported_invalid_txs: Mutex<Vec<(OLTxId, MempoolTxInvalidReason)>>,
    fail_mode: Mutex<MockMempoolFailMode>,
}

/// Failure injection mode for [`MockMempoolProvider`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum MockMempoolFailMode {
    #[default]
    None,
    GetTransactions,
    ReportInvalidTransactions,
}

impl MockMempoolProvider {
    /// Create a new empty mock mempool provider.
    pub(crate) fn new() -> Self {
        Self {
            transactions: Mutex::new(Vec::new()),
            report_call_count: AtomicUsize::new(0),
            last_reported_invalid_txs: Mutex::new(Vec::new()),
            fail_mode: Mutex::new(MockMempoolFailMode::None),
        }
    }

    /// Add a transaction to the mock mempool.
    pub(crate) fn add_transaction(&self, txid: OLTxId, tx: OLTransaction) {
        self.transactions.lock().unwrap().push((txid, tx));
    }

    /// Configures a failure injection mode.
    pub(crate) fn set_fail_mode(&self, fail_mode: MockMempoolFailMode) {
        *self.fail_mode.lock().unwrap() = fail_mode;
    }

    /// Returns the number of times `report_invalid_transactions` was called.
    pub(crate) fn report_call_count(&self) -> usize {
        self.report_call_count.load(Ordering::Relaxed)
    }

    /// Returns the most recent invalid-tx payload passed to `report_invalid_transactions`.
    pub(crate) fn last_reported_invalid_txs(&self) -> Vec<(OLTxId, MempoolTxInvalidReason)> {
        self.last_reported_invalid_txs.lock().unwrap().clone()
    }
}

#[async_trait]
impl MempoolProvider for MockMempoolProvider {
    async fn get_transactions(
        &self,
        limit: usize,
    ) -> BlockAssemblyResult<Vec<(OLTxId, OLTransaction)>> {
        if *self.fail_mode.lock().unwrap() == MockMempoolFailMode::GetTransactions {
            return Err(crate::BlockAssemblyError::Mempool(
                OLMempoolError::ServiceClosed(
                    "mock mempool: get_transactions injected failure".to_string(),
                ),
            ));
        }

        let txs = self.transactions.lock().unwrap();
        Ok(txs.iter().take(limit).cloned().collect())
    }

    async fn report_invalid_transactions(
        &self,
        txs: &[(OLTxId, MempoolTxInvalidReason)],
    ) -> BlockAssemblyResult<()> {
        self.report_call_count.fetch_add(1, Ordering::Relaxed);
        *self.last_reported_invalid_txs.lock().unwrap() = txs.to_vec();

        if *self.fail_mode.lock().unwrap() == MockMempoolFailMode::ReportInvalidTransactions {
            return Err(crate::BlockAssemblyError::Mempool(
                OLMempoolError::ServiceClosed(
                    "mock mempool: report_invalid_transactions injected failure".to_string(),
                ),
            ));
        }

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
    ) -> BlockAssemblyResult<Vec<(OLTxId, OLTransaction)>> {
        MempoolProvider::get_transactions(self.as_ref(), limit).await
    }

    async fn report_invalid_transactions(
        &self,
        txs: &[(OLTxId, MempoolTxInvalidReason)],
    ) -> BlockAssemblyResult<()> {
        MempoolProvider::report_invalid_transactions(self.as_ref(), txs).await
    }
}

/// State provider test double that always fails state lookup.
pub(crate) struct FailingStateProvider;

impl StateProvider for FailingStateProvider {
    type State = MemoryStateBaseLayer;
    type Error = DbError;

    #[expect(
        clippy::manual_async_fn,
        reason = "Keep explicit Future return shape in this test double implementation."
    )]
    fn get_state_for_tip_async(
        &self,
        _tip: OLBlockCommitment,
    ) -> impl Future<Output = Result<Option<Self::State>, Self::Error>> + Send {
        async {
            Err(DbError::Other(
                "injected state provider failure".to_string(),
            ))
        }
    }

    fn get_state_for_tip_blocking(
        &self,
        _tip: OLBlockCommitment,
    ) -> Result<Option<Self::State>, Self::Error> {
        Err(DbError::Other(
            "injected state provider failure".to_string(),
        ))
    }
}

/// Concrete block assembly context for tests using mock implementations.
pub(crate) type BlockAssemblyContextImpl =
    BlockAssemblyContext<Arc<MockMempoolProvider>, OLStateManagerProviderImpl>;

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
            .mmr_index()
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
}

// ===== Mempool Transaction Builder =====

/// Builder for creating mempool OL transactions for snark account updates.
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
    output_messages: Vec<OutputMessage>,
}

/// Builder for creating mempool OL transactions for generic account messages.
///
/// Produces a valid GAM transaction shape:
/// - payload target matches message destination
/// - exactly one message in effects
/// - zero message value
/// - no transfers
/// - no sequence-number field (GAM payloads are non-sequenced)
pub(crate) struct MempoolGamTxBuilder {
    target: AccountId,
    data: Vec<u8>,
}

impl MempoolGamTxBuilder {
    /// Creates a new GAM builder targeting the given account.
    pub(crate) fn new(target: AccountId) -> Self {
        Self {
            target,
            data: Vec::new(),
        }
    }

    /// Sets message payload bytes.
    pub(crate) fn with_data(mut self, data: Vec<u8>) -> Self {
        self.data = data;
        self
    }

    /// Builds the mempool transaction.
    pub(crate) fn build(self) -> OLTransaction {
        OLTransaction::new(
            OLTransactionData::new_gam(self.target, self.data),
            TxProofs::new_empty(),
        )
    }
}

/// Constructs a bridge-gateway withdrawal output message.
pub(crate) fn withdrawal_output_message(
    amount_sats: u64,
    destination_desc: Vec<u8>,
    withdrawal_fee_sats: u32,
) -> OutputMessage {
    let withdrawal_data = WithdrawalMsgData::new(withdrawal_fee_sats, destination_desc, u32::MAX)
        .expect("valid withdrawal data");
    let encoded_body = encode_to_vec(&withdrawal_data).expect("encode withdrawal body");
    let withdrawal_msg = OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_body).expect("msg format");
    let payload = MsgPayload::new(
        BitcoinAmount::from_sat(amount_sats),
        withdrawal_msg.to_vec(),
    );
    OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, payload)
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
            output_messages: Vec::new(),
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

    /// Sets simple output transfers as `(destination_account, value_sats)`.
    ///
    /// These are encoded as output messages with empty payload data.
    pub(crate) fn with_outputs(mut self, outputs: Vec<(AccountId, u64)>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Appends a withdrawal message routed to the bridge-gateway account.
    pub(crate) fn with_withdrawal(mut self, amount_sats: u64, destination_desc: Vec<u8>) -> Self {
        self.output_messages.push(withdrawal_output_message(
            amount_sats,
            destination_desc,
            DEFAULT_OPERATOR_FEE,
        ));
        self
    }

    /// Appends `count` withdrawal messages routed to the bridge-gateway account.
    pub(crate) fn with_withdrawals(
        mut self,
        count: usize,
        amount_sats: u64,
        destination_desc: Vec<u8>,
    ) -> Self {
        for _ in 0..count {
            self.output_messages.push(withdrawal_output_message(
                amount_sats,
                destination_desc.clone(),
                DEFAULT_OPERATOR_FEE,
            ));
        }
        self
    }

    /// Builds the mempool transaction.
    pub(crate) fn build(self) -> OLTransaction {
        // Use a random inner state from proptest
        let mut runner = TestRunner::default();
        let sau_payload = ol_test_utils::sau_tx_payload_strategy()
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let inner_state = sau_payload
            .operation()
            .update()
            .proof_state()
            .inner_state_root();
        let proof_state = SauTxProofState::new(self.new_msg_idx, inner_state);
        let update_data = SauTxUpdateData::new(self.seq_no, proof_state, vec![]);

        let ledger_refs = if self.l1_claims.is_empty() {
            SauTxLedgerRefs::new_empty()
        } else {
            let claim_list =
                ClaimList::new(self.l1_claims).expect("snark update has too many ASM claims");
            SauTxLedgerRefs::new_with_claims(claim_list)
        };

        let operation_data =
            SauTxOperationData::new(update_data, self.processed_messages, ledger_refs);
        let payload = TransactionPayload::SnarkAccountUpdate(SauTxPayload::new(
            self.account_id,
            operation_data,
        ));

        // Build effects: empty by default. `output_messages()` take precedence;
        // otherwise we synthesize plain value outputs from `with_outputs()`.
        let output_messages = if !self.output_messages.is_empty() {
            self.output_messages
        } else if self.outputs.is_empty() {
            Vec::new()
        } else {
            self.outputs
                .into_iter()
                .map(|(dest, value_sats)| {
                    let payload = MsgPayload::new(BitcoinAmount::from_sat(value_sats), vec![]);
                    OutputMessage::new(dest, payload)
                })
                .collect()
        };

        let mut effects = strata_acct_types::TxEffects::default();
        for msg in output_messages {
            effects.push_message(
                msg.dest(),
                msg.payload().value().to_sat(),
                msg.payload().data().to_vec(),
            );
        }

        let data = OLTransactionData::new(payload, effects);
        let update_proof = prop::collection::vec(any::<u8>(), 0..64)
            .new_tree(&mut runner)
            .unwrap()
            .current();
        let proofs = TxProofs::new(ProofSatisfierList::single(update_proof), None);
        OLTransaction::new(data, proofs)
    }
}

pub(crate) fn add_snark_account_to_state(
    state: &mut impl IStateAccessorMut,
    account_id: AccountId,
    state_root_seed: u8,
    initial_balance: u64,
) {
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(initial_balance),
        NewAccountTypeState::Snark {
            update_vk: PredicateKey::always_accept(),
            initial_state_root: test_hash(state_root_seed),
        },
    );
    state.create_new_account(account_id, new_acct).unwrap();
}

/// Inserts inbox messages into a snark account's state MMR.
pub(crate) fn insert_inbox_messages_into_state(
    state: &mut impl IStateAccessorMut,
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

/// Create test parent header by executing genesis block.
pub(crate) fn create_test_parent_header() -> strata_ol_chain_types_new::OLBlockHeader {
    let mut runner = TestRunner::default();
    let timestamp = (1000000u64..2000000u64)
        .new_tree(&mut runner)
        .unwrap()
        .current();

    let genesis_info = BlockInfo::new_genesis(timestamp);
    let mut temp_state = create_test_genesis_state();
    let genesis_context = BlockContext::new(&genesis_info, None);
    let genesis_components = BlockComponents::new_empty();
    let genesis_output =
        stf_construct_block(&mut temp_state, genesis_context, genesis_components).unwrap();
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

// ===== Test Environment Builder (Commit 2) =====

/// Setup ASM state with L1 manifests in storage.
///
/// Creates and stores ASM manifests for L1 blocks from height `start` to `end` (inclusive),
/// and stores an ASM state at the highest L1 block.
///
/// Returns the L1BlockCommitment for the highest block.
pub(crate) async fn setup_asm_state_with_l1_manifests(
    storage: &NodeStorage,
    start: L1Height,
    end: L1Height,
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
        )
        .expect("test manifest should be valid");

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
    let l1_commitment = L1BlockCommitment::new(end, last_blkid);

    // Create minimal ASM state for testing
    let pow_state = HeaderVerificationState::init(L1Anchor {
        block: l1_commitment,
        next_target: 0,
        epoch_start_timestamp: 0,
        network: Network::Bitcoin,
    });
    let history_accumulator = AsmHistoryAccumulatorState::new(0);
    let chain_view = ChainViewState {
        pow_state,
        history_accumulator,
    };
    let anchor_state = AnchorState {
        magic: AnchorState::magic_ssz(MagicBytes::from(*b"ALPN")),
        chain_view,
        sections: Default::default(),
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

/// Typed account setup used by test environment builders.
#[derive(Clone)]
pub(crate) struct TestAccount {
    id: AccountId,
    balance: u64,
    inbox: Vec<MessageEntry>,
}

impl TestAccount {
    /// Creates a test account with a balance and empty inbox.
    pub(crate) fn new(id: AccountId, balance: u64) -> Self {
        Self {
            id,
            balance,
            inbox: Vec::new(),
        }
    }

    /// Adds pre-seeded inbox messages for this account.
    pub(crate) fn with_inbox(mut self, messages: Vec<MessageEntry>) -> Self {
        self.inbox = messages;
        self
    }
}

/// Storage fixture layer for block assembly tests.
pub(crate) struct TestStorageFixture {
    storage: Arc<NodeStorage>,
    l1_header_refs: Vec<(L1Height, AccumulatorClaim)>,
    inbox_message_claims: Vec<(AccountId, Vec<AccumulatorClaim>)>,
}

impl TestStorageFixture {
    /// Creates a new fixture from storage.
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self {
            storage,
            l1_header_refs: Vec::new(),
            inbox_message_claims: Vec::new(),
        }
    }

    /// Sets seeded L1 header refs and inbox message claims produced during fixture setup.
    pub(crate) fn with_seeded_claims(
        mut self,
        l1_header_refs: Vec<(L1Height, AccumulatorClaim)>,
        inbox_message_claims: Vec<(AccountId, Vec<AccumulatorClaim>)>,
    ) -> Self {
        self.l1_header_refs = l1_header_refs;
        self.inbox_message_claims = inbox_message_claims;
        self
    }

    /// Returns storage handle for lower-level tests.
    pub(crate) fn storage(&self) -> &Arc<NodeStorage> {
        &self.storage
    }

    /// Returns configured L1 header refs keyed by L1 height.
    pub(crate) fn l1_header_refs(&self) -> &[(L1Height, AccumulatorClaim)] {
        &self.l1_header_refs
    }

    /// Returns an L1 header ref for a specific L1 height.
    pub(crate) fn l1_header_ref(&self, height: L1Height) -> Option<AccumulatorClaim> {
        self.l1_header_refs
            .iter()
            .find(|(h, _)| *h == height)
            .map(|(_, claim)| claim.clone())
    }

    /// Returns inbox message claims for a specific account.
    pub(crate) fn inbox_message_claims_for_account(
        &self,
        account_id: AccountId,
    ) -> &[AccumulatorClaim] {
        self.inbox_message_claims
            .iter()
            .find(|(id, _)| *id == account_id)
            .map(|(_, claims)| claims.as_slice())
            .unwrap_or(&[])
    }
}

/// Behavior-level test environment for block assembly.
pub(crate) struct TestEnv {
    fixture: Arc<TestStorageFixture>,
    ctx: Arc<BlockAssemblyContextImpl>,
    mempool: Arc<MockMempoolProvider>,
    sequencer_config: SequencerConfig,
    epoch_sealing_policy: FixedSlotSealing,
    parent_commitment: OLBlockCommitment,
}

impl TestEnv {
    /// Creates a behavior-level env from a seeded fixture and parent commitment.
    pub(crate) fn from_fixture(
        fixture: Arc<TestStorageFixture>,
        parent_commitment: OLBlockCommitment,
    ) -> Self {
        let (ctx, mempool) = create_test_block_assembly_context(fixture.storage().clone());
        Self {
            fixture,
            ctx: Arc::new(ctx),
            mempool,
            sequencer_config: SequencerConfig::default(),
            epoch_sealing_policy: FixedSlotSealing::new(TEST_SLOTS_PER_EPOCH),
            parent_commitment,
        }
    }

    /// Returns parent commitment used by default assembly entrypoints.
    pub(crate) fn parent_commitment(&self) -> OLBlockCommitment {
        self.parent_commitment
    }

    /// Returns storage handle for storage-backed test setup helpers.
    pub(crate) fn storage(&self) -> &Arc<NodeStorage> {
        self.fixture.storage()
    }

    /// Returns the block assembly context bound to this environment.
    ///
    /// Prefer behavior-level helpers (`generate_block_template` /
    /// `construct_block`) unless a test needs direct context APIs.
    pub(crate) fn ctx(&self) -> &BlockAssemblyContextImpl {
        self.ctx.as_ref()
    }

    /// Returns shared block assembly context handle.
    pub(crate) fn ctx_arc(&self) -> Arc<BlockAssemblyContextImpl> {
        self.ctx.clone()
    }

    /// Returns mock mempool handle for injection/inspection tests.
    pub(crate) fn mempool(&self) -> &MockMempoolProvider {
        self.mempool.as_ref()
    }

    /// Returns shared mock mempool handle.
    pub(crate) fn mempool_arc(&self) -> Arc<MockMempoolProvider> {
        self.mempool.clone()
    }

    /// Appends inbox messages to the storage MMR for a given account and returns MMR indices.
    ///
    /// This is a runtime storage update helper for multi-block tests where
    /// proofs in a later block must reference newly-added inbox entries.
    pub(crate) fn append_inbox_messages(
        &self,
        account_id: AccountId,
        messages: impl IntoIterator<Item = MessageEntry>,
    ) -> Vec<u64> {
        let mut inbox_mmr = StorageInboxMmr::new(self.storage().as_ref(), account_id);
        inbox_mmr.add_messages(messages)
    }

    /// Returns configured L1 header refs keyed by L1 height.
    pub(crate) fn l1_header_refs(&self) -> &[(L1Height, AccumulatorClaim)] {
        self.fixture.l1_header_refs()
    }

    /// Returns an L1 header ref for a specific L1 height.
    pub(crate) fn l1_header_ref(&self, height: L1Height) -> Option<AccumulatorClaim> {
        self.fixture.l1_header_ref(height)
    }

    /// Returns inbox message claims for a specific account.
    pub(crate) fn inbox_message_claims_for_account(
        &self,
        account_id: AccountId,
    ) -> &[AccumulatorClaim] {
        self.fixture.inbox_message_claims_for_account(account_id)
    }

    /// Returns sequencer config.
    pub(crate) fn sequencer_config(&self) -> &SequencerConfig {
        &self.sequencer_config
    }

    /// Returns epoch sealing policy.
    pub(crate) fn epoch_sealing_policy(&self) -> &FixedSlotSealing {
        &self.epoch_sealing_policy
    }

    /// Returns a block-generation config rooted at this environment's parent commitment.
    fn parent_config(&self) -> BlockGenerationConfig {
        BlockGenerationConfig::new(self.parent_commitment())
    }

    /// Generates block template from mempool using current parent commitment and empty parent DA.
    pub(crate) async fn generate_block_template(&self) -> BlockAssemblyResult<BlockTemplateResult> {
        self.generate_block_template_with_da(AccumulatedDaData::new_empty())
            .await
    }

    /// Generates block template from mempool using current parent commitment and explicit parent
    /// DA.
    pub(crate) async fn generate_block_template_with_da(
        &self,
        parent_da: AccumulatedDaData,
    ) -> BlockAssemblyResult<BlockTemplateResult> {
        let config = self.parent_config();
        generate_block_template_inner(
            self.ctx(),
            self.epoch_sealing_policy(),
            self.sequencer_config(),
            config,
            parent_da,
        )
        .await
    }

    /// Constructs a block directly from explicit txs using current parent commitment and empty
    /// parent DA.
    pub(crate) async fn construct_block(
        &self,
        txs: impl IntoIterator<Item = (OLTxId, OLTransaction)>,
    ) -> BlockAssemblyResult<ConstructBlockOutput<MemoryStateBaseLayer>> {
        self.construct_block_with_da(txs, AccumulatedDaData::new_empty())
            .await
    }

    /// Constructs an empty block using current parent commitment and empty parent DA.
    pub(crate) async fn construct_empty_block(
        &self,
    ) -> BlockAssemblyResult<ConstructBlockOutput<MemoryStateBaseLayer>> {
        self.construct_block(iter::empty::<(OLTxId, OLTransaction)>())
            .await
    }

    /// Constructs an empty block using current parent commitment and explicit parent DA.
    pub(crate) async fn construct_empty_block_with_da(
        &self,
        parent_da: AccumulatedDaData,
    ) -> BlockAssemblyResult<ConstructBlockOutput<MemoryStateBaseLayer>> {
        self.construct_block_with_da(iter::empty::<(OLTxId, OLTransaction)>(), parent_da)
            .await
    }

    /// Constructs a block directly from explicit txs and parent DA using current parent commitment.
    pub(crate) async fn construct_block_with_da(
        &self,
        txs: impl IntoIterator<Item = (OLTxId, OLTransaction)>,
        parent_da: AccumulatedDaData,
    ) -> BlockAssemblyResult<ConstructBlockOutput<MemoryStateBaseLayer>> {
        let config = self.parent_config();
        assemble_block_with_txs(
            self.ctx(),
            self.epoch_sealing_policy(),
            &config,
            txs.into_iter().collect(),
            parent_da,
        )
        .await
    }

    /// Persists assembled output as the next parent block/state and advances parent commitment.
    pub(crate) async fn persist(
        &mut self,
        output: &ConstructBlockOutput<MemoryStateBaseLayer>,
    ) -> OLBlockCommitment {
        let header = output.template.header().clone();
        let commitment = OLBlockCommitment::new(header.slot(), header.compute_blkid());
        let (block, post_state) = block_and_post_state_from_output(output);

        self.storage()
            .ol_block()
            .put_block_data_async(block)
            .await
            .expect("store assembled block");
        self.storage()
            .ol_state()
            .put_toplevel_ol_state_async(commitment, post_state.into_inner())
            .await
            .expect("store assembled post-state");

        self.parent_commitment = commitment;
        commitment
    }

    /// Stores an OL block in runtime storage.
    ///
    /// Use this for tests that need runtime block injection without reaching into
    /// raw storage plumbing from test bodies.
    pub(crate) async fn put_block(&self, block: OLBlock) {
        self.storage()
            .ol_block()
            .put_block_data_async(block)
            .await
            .expect("store block");
    }
}

/// Converts assembled output into persisted artifacts: `(OLBlock, post_state)`.
pub(crate) fn block_and_post_state_from_output(
    output: &ConstructBlockOutput<MemoryStateBaseLayer>,
) -> (OLBlock, MemoryStateBaseLayer) {
    let header = output.template.header().clone();
    let signed_header = SignedOLBlockHeader::new(header, Buf64::zero());
    let block = OLBlock::new(signed_header, output.template.body().clone());
    (block, output.post_state.clone())
}

/// Builder for seeded storage fixtures used by block assembly tests.
#[derive(Default)]
pub(crate) struct TestStorageFixtureBuilder {
    parent_slot: Option<u64>,
    l1_manifest_height_range: Option<RangeInclusive<L1Height>>,
    l1_header_ref_heights: Vec<L1Height>,
    expected_l1_header_ref_indices: Vec<(L1Height, u64)>,
    expected_inbox_message_indices: Vec<(AccountId, Vec<u64>)>,
    accounts: Vec<TestAccount>,
}

impl TestStorageFixtureBuilder {
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

    /// Adds a configured account fixture.
    pub(crate) fn with_account(mut self, account: TestAccount) -> Self {
        self.accounts.push(account);
        self
    }

    /// Adds multiple configured account fixtures.
    pub(crate) fn with_accounts(mut self, accounts: impl IntoIterator<Item = TestAccount>) -> Self {
        self.accounts.extend(accounts);
        self
    }

    /// Stores L1 manifests in ASM storage for block's L1 update fetching.
    pub(crate) fn with_l1_manifest_height_range(mut self, range: RangeInclusive<L1Height>) -> Self {
        self.l1_manifest_height_range = Some(range);
        self
    }

    /// Seeds L1 header refs for the provided L1 heights.
    pub(crate) fn with_l1_header_refs(
        mut self,
        heights: impl IntoIterator<Item = L1Height>,
    ) -> Self {
        self.l1_header_ref_heights = heights.into_iter().collect();
        self
    }

    /// Asserts exact seeded L1 header ref indices as `(l1_height, expected_mmr_index)` entries.
    pub(crate) fn with_expected_l1_header_ref_indices(
        mut self,
        expected: impl IntoIterator<Item = (L1Height, u64)>,
    ) -> Self {
        self.expected_l1_header_ref_indices = expected.into_iter().collect();
        self
    }

    /// Asserts exact seeded inbox message indices as `(account_id, [expected_mmr_index])` entries.
    ///
    /// Inbox messages come from account fixtures (`TestAccount::with_inbox`).
    pub(crate) fn with_expected_inbox_message_indices(
        mut self,
        indices: impl IntoIterator<Item = (AccountId, Vec<u64>)>,
    ) -> Self {
        self.expected_inbox_message_indices = indices.into_iter().collect();
        self
    }

    /// Builds and seeds the storage fixture.
    ///
    /// Returns `(fixture, parent_commitment)` for advanced tests that need lower layers directly.
    pub(crate) async fn build_fixture(self) -> (Arc<TestStorageFixture>, OLBlockCommitment) {
        let fixture = TestStorageFixture::new(create_test_storage());

        // Setup ASM state with L1 manifests if configured.
        if let Some(range) = &self.l1_manifest_height_range {
            let min_height = *range.start();
            let max_height = *range.end();
            setup_asm_state_with_l1_manifests(fixture.storage().as_ref(), min_height, max_height)
                .await;
        }

        // Create genesis state
        let mut state = create_test_genesis_state();

        // Add snark accounts
        let mut inbox_message_claims = Vec::new();
        for (i, account) in self.accounts.iter().enumerate() {
            add_snark_account_to_state(&mut state, account.id, i as u8 + 1, account.balance);
            if !account.inbox.is_empty() {
                insert_inbox_messages_into_state(&mut state, account.id, &account.inbox);

                let mut inbox_mmr = StorageInboxMmr::new(fixture.storage().as_ref(), account.id);
                let indices = inbox_mmr.add_messages(account.inbox.clone());
                if let Some((_, expected_indices)) = self
                    .expected_inbox_message_indices
                    .iter()
                    .find(|(account_id, _)| *account_id == account.id)
                {
                    assert_eq!(
                        expected_indices.len(),
                        indices.len(),
                        "expected inbox ref count mismatch for account {:?}",
                        account.id
                    );
                    for (position, (expected_idx, actual_idx)) in
                        expected_indices.iter().zip(indices.iter()).enumerate()
                    {
                        assert_eq!(
                            *expected_idx, *actual_idx,
                            "seeded inbox ref index mismatch for account {:?} at position {}",
                            account.id, position
                        );
                    }
                }
                let claims = build_inbox_claims_for_messages(&account.inbox, &indices);
                inbox_message_claims.push((account.id, claims));
            }
        }

        for (account_id, _) in &self.expected_inbox_message_indices {
            let is_seeded = inbox_message_claims
                .iter()
                .any(|(seeded_account_id, _)| seeded_account_id == account_id);
            assert!(
                is_seeded,
                "inbox message indices configured for account {:?}, but no inbox messages were seeded for that account",
                account_id
            );
        }

        // Seed requested L1 header refs into both state claims and the ASM MMR.
        let mut l1_heights = self.l1_header_ref_heights.clone();
        l1_heights.extend(
            self.expected_l1_header_ref_indices
                .iter()
                .map(|(height, _)| *height),
        );
        l1_heights.sort_unstable();
        l1_heights.dedup();

        let seeded_l1_header_refs = if !l1_heights.is_empty() {
            let asm_manifests = create_test_manifests_for_heights(&l1_heights);
            let claims_by_height = setup_manifests_in_state_and_storage(
                fixture.storage().as_ref(),
                &mut state,
                asm_manifests,
            );

            for (height, expected_idx) in &self.expected_l1_header_ref_indices {
                let (_, claim) = claims_by_height
                    .iter()
                    .find(|(h, _)| h == height)
                    .unwrap_or_else(|| panic!("missing seeded claim for l1 height {height}"));
                assert_eq!(
                    claim.idx(),
                    *expected_idx,
                    "seeded claim index mismatch for l1 height {height}"
                );
            }

            claims_by_height
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

                // Create genesis manifest at height 1 (when last_l1_height is 0)
                let genesis_manifest = AsmManifest::new(
                    1,
                    L1BlockId::from(Buf32::zero()),
                    WtxidsRoot::from(Buf32::zero()),
                    vec![],
                )
                .expect("test manifest should be valid");
                let components = BlockComponents::new_manifests(vec![genesis_manifest]);

                let block_context = BlockContext::new(&block_info, None);
                let construct_output = stf_construct_block(&mut state, block_context, components)
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

            fixture
                .storage()
                .ol_state()
                .put_toplevel_ol_state_async(commitment, parent_state.into_inner())
                .await
                .expect("Failed to store parent OL state");

            fixture
                .storage()
                .ol_block()
                .put_block_data_async(parent_block)
                .await
                .expect("Failed to store parent block");

            commitment
        } else {
            // No parent slot - return null commitment for genesis testing
            let null_commitment = OLBlockCommitment::null();
            fixture
                .storage()
                .ol_state()
                .put_toplevel_ol_state_async(null_commitment, state.into_inner())
                .await
                .expect("Failed to store genesis OL state at null commitment");
            null_commitment
        };

        let fixture =
            Arc::new(fixture.with_seeded_claims(seeded_l1_header_refs, inbox_message_claims));
        (fixture, parent_commitment)
    }
}

/// Create deterministic test manifests for specific L1 heights.
///
/// Returns manifests that can be used to populate both state and storage MMRs.
fn create_test_manifests_for_heights(heights: &[L1Height]) -> Vec<AsmManifest> {
    heights
        .iter()
        .copied()
        .map(|height| {
            let mut blkid_bytes = [0u8; 32];
            blkid_bytes[0] = height as u8;
            blkid_bytes[1] = (height >> 8) as u8;
            AsmManifest::new(
                height,
                L1BlockId::from(Buf32::from(blkid_bytes)),
                WtxidsRoot::from(Buf32::zero()),
                vec![],
            )
            .expect("test manifest should be valid")
        })
        .collect()
}

/// Setup manifests in both storage MMR and state's manifest MMR.
///
/// This ensures consistency between proof generation (uses storage MMR) and
/// verification (uses state's manifest MMR).
///
/// Returns `(l1_height, claim)` pairs for each inserted manifest.
fn setup_manifests_in_state_and_storage(
    storage: &NodeStorage,
    state: &mut impl IStateAccessorMut,
    manifests: Vec<AsmManifest>,
) -> Vec<(L1Height, AccumulatorClaim)> {
    let mmr_handle = storage.mmr_index().as_ref().get_handle(MmrId::Asm);

    let mut claims_by_l1_height = Vec::with_capacity(manifests.len());

    for manifest in manifests {
        // Compute manifest hash
        let manifest_hash: Hash = manifest.compute_hash().into();

        // Add to storage MMR (for proof generation)
        let leaf_idx = mmr_handle.append_leaf_blocking(manifest_hash).unwrap();
        let claim = AccumulatorClaim::new(leaf_idx, manifest_hash);

        // Add to state's manifest MMR (for verification)
        let height = manifest.height();
        state.append_manifest(height, manifest);
        claims_by_l1_height.push((height, claim));
    }

    claims_by_l1_height.sort_by_key(|(height, _)| *height);
    claims_by_l1_height
}

fn build_inbox_claims_for_messages(
    messages: &[MessageEntry],
    indices: &[u64],
) -> Vec<AccumulatorClaim> {
    assert_eq!(
        messages.len(),
        indices.len(),
        "messages/indices length mismatch while building inbox claims"
    );
    indices
        .iter()
        .zip(messages.iter())
        .map(|(&idx, msg)| {
            let hash = <MessageEntry as TreeHash>::tree_hash_root(msg).into_inner();
            AccumulatorClaim::new(idx, hash)
        })
        .collect()
}

/// Create test BlockAssemblyContext with mock providers.
///
/// Returns the context. Use `ctx.mempool_provider()` to add transactions to the mock mempool.
pub(crate) fn create_test_block_assembly_context(
    storage: Arc<NodeStorage>,
) -> (BlockAssemblyContextImpl, Arc<MockMempoolProvider>) {
    let mempool_provider = Arc::new(MockMempoolProvider::new());
    let state_provider = OLStateManagerProviderImpl::new(storage.ol_state().clone());
    let ctx = BlockAssemblyContext::new(storage, mempool_provider.clone(), state_provider, 0);
    (ctx, mempool_provider)
}

// ===== Result Inspection Helpers =====

/// Returns included txids in block order.
pub(crate) fn included_txids(template: &FullBlockTemplate) -> Vec<OLTxId> {
    template
        .body()
        .tx_segment()
        .expect("tx segment")
        .txs()
        .iter()
        .map(OLTransaction::compute_txid)
        .collect()
}

/// Returns post-state root committed in the block template header.
pub(crate) fn template_state_root(template: &FullBlockTemplate) -> Hash {
    *template.header().state_root()
}

/// Returns decoded withdrawal intent logs from accumulated DA logs.
pub(crate) fn withdrawal_intents(
    output: &ConstructBlockOutput<MemoryStateBaseLayer>,
) -> Vec<SimpleWithdrawalIntentLogData> {
    output
        .accumulated_da
        .logs()
        .iter()
        .filter(|log| log.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
        .filter_map(|log| decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload()).ok())
        .collect()
}

/// Returns accumulated DA with `n` seeded dummy logs.
pub(crate) fn seeded_da(n: usize) -> AccumulatedDaData {
    let logs = (0..n)
        .map(|i| OLLog::new(AccountSerial::from(i as u32), vec![]))
        .collect();
    AccumulatedDaData::new(EpochDaAccumulator::default(), logs)
}

// ===== Assembly Pipeline Helper =====

/// Assembles a block for `config` using tx list and parent DA snapshot.
pub(crate) async fn assemble_block_with_txs(
    ctx: &BlockAssemblyContextImpl,
    epoch_sealing_policy: &FixedSlotSealing,
    config: &BlockGenerationConfig,
    txs: Vec<(OLTxId, OLTransaction)>,
    parent_da: AccumulatedDaData,
) -> BlockAssemblyResult<ConstructBlockOutput<MemoryStateBaseLayer>> {
    let parent_state = ctx
        .fetch_state_for_tip(config.parent_block_commitment())
        .await?
        .expect("parent state should exist");

    let (block_slot, block_epoch) =
        calculate_block_slot_and_epoch(&config.parent_block_commitment(), parent_state.as_ref());

    construct_block(
        ctx,
        epoch_sealing_policy,
        config,
        parent_state,
        block_slot,
        block_epoch,
        txs,
        parent_da,
    )
    .await
}
