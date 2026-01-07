//! Test utilities for mempool tests.

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, RwLock},
};

use proptest::{
    arbitrary,
    prelude::*,
    strategy::{Strategy, ValueTree},
    test_runner::TestRunner,
};
use strata_acct_types::{AccountId, BitcoinAmount};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_identifiers::{Buf32, Hash, OLBlockCommitment, OLBlockId};
use strata_ledger_types::{
    AccountTypeState, IAccountStateMut, ISnarkAccountStateMut, IStateAccessor, NewAccountData,
};
use strata_ol_chain_types_new::{TransactionAttachment, test_utils as ol_test_utils};
use strata_ol_state_types::{OLSnarkAccountState, OLState, StateProvider};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{Seqno, SnarkAccountUpdate, UpdateOperationData};
use strata_storage::{NodeStorage, create_node_storage};
use threadpool::ThreadPool;

use crate::{
    OLMempoolSnarkAcctUpdateTxPayload, OLMempoolTransaction, OLMempoolTxPayload,
    state::MempoolContext, types::OLMempoolConfig,
};

/// Create a test account ID using proptest strategy.
pub(crate) fn create_test_account_id() -> AccountId {
    let mut runner = TestRunner::default();
    arbitrary::any::<[u8; 32]>()
        .new_tree(&mut runner)
        .unwrap()
        .current()
        .into()
}

/// Create a test account ID with a specific ID byte for deterministic testing.
pub(crate) fn create_test_account_id_with(id: u8) -> AccountId {
    let mut bytes = [0u8; 32];
    bytes[0] = id;
    AccountId::new(bytes)
}

/// Create a test transaction attachment using proptest strategy.
pub(crate) fn create_test_attachment() -> TransactionAttachment {
    let mut runner = TestRunner::default();
    ol_test_utils::transaction_attachment_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current()
}

/// Create a test snark account update (base_update only, no accumulator proofs).
pub(crate) fn create_test_snark_update() -> SnarkAccountUpdate {
    // Use ol-chain-types strategy and extract base_update
    let mut runner = TestRunner::default();
    let full_payload = ol_test_utils::snark_account_update_tx_payload_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();

    full_payload.update_container.base_update
}

/// Create a test transaction attachment with optional min/max slots.
pub(crate) fn create_test_attachment_with_slots(
    min_slot: Option<u64>,
    max_slot: Option<u64>,
) -> TransactionAttachment {
    TransactionAttachment::new(min_slot, max_slot)
}

/// Create a test OL block commitment.
///
/// Uses a simple block ID pattern (slot value in first byte) for testing.
/// The block ID doesn't affect validation logic but using a non-null ID is better practice.
pub(crate) fn create_test_block_commitment(slot: u64) -> OLBlockCommitment {
    let mut bytes = [0u8; 32];
    // Use slot value in first byte to make block ID unique per slot
    bytes[0] = (slot & 0xFF) as u8;
    OLBlockCommitment::new(slot, OLBlockId::from(Buf32::new(bytes)))
}

/// Create a test snark account update payload.
pub(crate) fn create_test_snark_payload() -> OLMempoolTxPayload {
    OLMempoolTxPayload::SnarkAccountUpdate(OLMempoolSnarkAcctUpdateTxPayload {
        target: create_test_account_id(),
        base_update: create_test_snark_update(),
    })
}

/// Create a test generic account message payload.
pub(crate) fn create_test_generic_payload() -> OLMempoolTxPayload {
    let mut runner = TestRunner::default();
    let gam_payload = ol_test_utils::gam_tx_payload_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();
    OLMempoolTxPayload::GenericAccountMessage(gam_payload)
}

/// Create a test mempool transaction from a payload.
pub(crate) fn create_test_mempool_tx(payload: OLMempoolTxPayload) -> OLMempoolTransaction {
    let attachment = create_test_attachment();
    match payload {
        OLMempoolTxPayload::SnarkAccountUpdate(snark_payload) => {
            OLMempoolTransaction::new_snark_account_update(
                snark_payload.target,
                snark_payload.base_update,
                attachment,
            )
        }
        OLMempoolTxPayload::GenericAccountMessage(gam_payload) => {
            OLMempoolTransaction::new_generic_account_message(
                *gam_payload.target(),
                gam_payload.payload().to_vec(),
                attachment,
            )
            .expect("Should create transaction")
        }
    }
}

/// Create a test OLState with an empty account for the given account ID.
///
/// Returns a state with an empty account for the given account ID at the specified slot.
/// This allows generic account message transactions to pass account existence checks.
pub(crate) fn create_test_ol_state_with_account(account_id: AccountId, slot: u64) -> OLState {
    let mut state = OLState::new_genesis();
    // Set the slot
    state.set_cur_slot(slot);
    // Create an empty account so it exists for validation
    let new_acct = NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Empty);
    state.create_new_account(account_id, new_acct).unwrap();
    state
}

/// Create a test OLState with a Snark account for testing SnarkAccountUpdate transactions.
///
/// # Arguments
/// * `account_id` - The account ID to create
/// * `seq_no` - The initial sequence number for the Snark account
/// * `slot` - The current slot for the state
///
/// # Returns
/// An `OLState` with the specified Snark account at the specified slot
pub(crate) fn create_test_ol_state_with_snark_account(
    account_id: AccountId,
    seq_no: u64,
    slot: u64,
) -> OLState {
    let mut state = OLState::new_genesis();
    // Set the slot
    state.set_cur_slot(slot);
    // Create a fresh snark account, then update its sequence number
    let pred_key = PredicateKey::always_accept();
    let snark_state = OLSnarkAccountState::new_fresh(pred_key, Hash::from([0u8; 32]));
    let new_acct =
        NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Snark(snark_state));
    state.create_new_account(account_id, new_acct).unwrap();

    // Update the sequence number using the mutable interface
    state
        .update_account(account_id, |account| {
            let snark_account = account.as_snark_account_mut().unwrap();
            snark_account.set_proof_state_directly(Hash::from([0u8; 32]), 0, Seqno::from(seq_no));
        })
        .unwrap();

    state
}

/// Create a test snark account update transaction.
pub(crate) fn create_test_snark_tx() -> OLMempoolTransaction {
    create_test_mempool_tx(create_test_snark_payload())
}

/// Create a test generic account message transaction.
/// Uses an attachment without slot restrictions (min_slot=None, max_slot=None).
pub(crate) fn create_test_generic_tx() -> OLMempoolTransaction {
    let attachment = create_test_attachment_with_slots(None, None);
    let payload = create_test_generic_payload();
    match payload {
        OLMempoolTxPayload::GenericAccountMessage(gam_payload) => {
            OLMempoolTransaction::new_generic_account_message(
                *gam_payload.target(),
                gam_payload.payload().to_vec(),
                attachment,
            )
            .expect("Should create transaction")
        }
        _ => panic!("Expected GenericAccountMessage"),
    }
}

/// Create a test generic account message transaction with attachment.
pub(crate) fn create_test_generic_tx_with_attachment(
    attachment: TransactionAttachment,
) -> OLMempoolTransaction {
    let target = create_test_account_id();
    let mut runner = TestRunner::default();
    let payload_strategy = prop::collection::vec(any::<u8>(), 10..100);
    let payload = payload_strategy.new_tree(&mut runner).unwrap().current();
    OLMempoolTransaction::new_generic_account_message(target, payload, attachment)
        .expect("Should create transaction")
}

/// Create a test generic account message transaction with specific slot bounds.
pub(crate) fn create_test_generic_tx_with_slots(
    min_slot: Option<u64>,
    max_slot: Option<u64>,
) -> OLMempoolTransaction {
    let attachment = create_test_attachment_with_slots(min_slot, max_slot);
    create_test_generic_tx_with_attachment(attachment)
}

/// Create a test generic account message transaction with a specific payload size.
pub(crate) fn create_test_generic_tx_with_size(
    target: AccountId,
    size: usize,
    attachment: TransactionAttachment,
) -> OLMempoolTransaction {
    let mut runner = TestRunner::default();
    let payload_strategy = prop::collection::vec(any::<u8>(), size..=size);
    let payload = payload_strategy.new_tree(&mut runner).unwrap().current();
    OLMempoolTransaction::new_generic_account_message(target, payload, attachment)
        .expect("Should create transaction")
}

/// Create a test transaction with a specific target account ID.
/// Uses the ID byte to create different account IDs, but the update content is randomly generated.
/// Uses an attachment without slot restrictions (min_slot=None, max_slot=None).
pub(crate) fn create_test_tx_with_id(id: u8) -> OLMempoolTransaction {
    let attachment = create_test_attachment_with_slots(None, None);
    OLMempoolTransaction::new_snark_account_update(
        create_test_account_id_with(id),
        create_test_snark_update(),
        attachment,
    )
}

/// Create a test snark transaction with a specific seq_no for deterministic ordering tests.
pub(crate) fn create_test_snark_tx_with_seq_no(
    account_id: u8,
    seq_no: u64,
) -> OLMempoolTransaction {
    create_test_snark_tx_with_seq_no_and_slots(account_id, seq_no, None, None)
}

/// Create a test snark transaction with a specific seq_no and slot bounds.
pub(crate) fn create_test_snark_tx_with_seq_no_and_slots(
    account_id: u8,
    seq_no: u64,
    min_slot: Option<u64>,
    max_slot: Option<u64>,
) -> OLMempoolTransaction {
    let mut runner = TestRunner::default();

    // Use attachment with specified slot bounds
    let attachment = create_test_attachment_with_slots(min_slot, max_slot);

    let full_payload = ol_test_utils::snark_account_update_tx_payload_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();

    let operation = UpdateOperationData::new(
        seq_no,
        full_payload
            .update_container
            .base_update
            .operation
            .new_proof_state(),
        full_payload
            .update_container
            .base_update
            .operation
            .processed_messages()
            .to_vec(),
        full_payload
            .update_container
            .base_update
            .operation
            .ledger_refs()
            .clone(),
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

    OLMempoolTransaction::new_snark_account_update(
        create_test_account_id_with(account_id),
        update,
        attachment,
    )
}

/// Set up a genesis state in the database for the given tip.
/// This is needed for tests that require state accessor.
/// Creates Snark accounts for common test account IDs (0-255) to allow
/// SnarkAccountUpdate transactions to pass validation.
/// Also creates empty accounts for any account IDs that GenericAccountMessage
/// transactions might use (they just need accounts to exist).
pub(crate) async fn setup_test_state_for_tip(storage: &NodeStorage, tip: OLBlockCommitment) {
    let mut state = OLState::new_genesis();
    state.set_cur_slot(tip.slot());

    // Create Snark accounts for common test account IDs (0-255)
    // Most tests use SnarkAccountUpdate transactions which require Snark accounts
    for id_byte in 0..=255u8 {
        let account_id = create_test_account_id_with(id_byte);
        let pred_key = PredicateKey::always_accept();
        let snark_state = OLSnarkAccountState::new_fresh(pred_key, Hash::from([0u8; 32]));
        let new_acct =
            NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Snark(snark_state));
        // Ignore errors if account already exists
        if state.create_new_account(account_id, new_acct).is_ok() {
            // Set initial seq_no to 0 for new Snark accounts
            let _ = state.update_account(account_id, |account| {
                let snark_account = account.as_snark_account_mut().unwrap();
                snark_account.set_proof_state_directly(Hash::from([0u8; 32]), 0, Seqno::from(0));
            });
        }
    }

    storage
        .ol_state()
        .put_toplevel_ol_state_async(tip, state)
        .await
        .expect("Failed to set up test state");
}

/// Create a test mempool context with specified configuration and provider.
pub(crate) fn create_test_context<P: StateProvider>(
    config: OLMempoolConfig,
    provider: Arc<P>,
) -> MempoolContext<P> {
    let pool = ThreadPool::new(1);

    // Create a minimal test storage using a test sled database
    // In real usage, this would be a full NodeStorage with all managers
    // For tests, we create a minimal storage since validation isn't called yet
    let test_db = get_test_sled_backend();
    let test_storage =
        Arc::new(create_node_storage(test_db, pool).expect("Failed to create test NodeStorage"));

    MempoolContext::new_with_provider(config, test_storage, provider)
}

/// Create an InMemoryStateProvider with initial test state at the given tip.
///
/// Creates a genesis state with Snark accounts for test account IDs (0-255).
pub(crate) fn create_test_state_provider(tip: OLBlockCommitment) -> InMemoryStateProvider {
    let state = create_test_ol_state_for_tip(tip.slot());
    InMemoryStateProvider::with_initial_state(tip, state)
}

/// Create a test OL state at a given slot with Snark accounts.
///
/// Creates a genesis state with Snark accounts for test account IDs (0-255).
pub(crate) fn create_test_ol_state_for_tip(slot: u64) -> OLState {
    let mut state = OLState::new_genesis();
    state.set_cur_slot(slot);

    // Create Snark accounts for common test account IDs (0-255)
    for id_byte in 0..=255u8 {
        let account_id = create_test_account_id_with(id_byte);
        let snark_state = OLSnarkAccountState::new_fresh(Hash::from([0u8; 32]));
        let new_acct =
            NewAccountData::new(BitcoinAmount::from(0), AccountTypeState::Snark(snark_state));
        if state.create_new_account(account_id, new_acct).is_ok() {
            let _ = state.update_account(account_id, |account| {
                let snark_account = account.as_snark_account_mut().unwrap();
                snark_account.set_proof_state_directly(Hash::from([0u8; 32]), 0, Seqno::from(0));
            });
        }
    }

    state
}

/// Create a test generic account message transaction for a specific account.
/// Uses an attachment without slot restrictions (min_slot=None, max_slot=None).
/// Uses a unique random payload to ensure unique transaction IDs.
pub(crate) fn create_test_generic_tx_for_account(account_id: u8) -> OLMempoolTransaction {
    let attachment = create_test_attachment_with_slots(None, None);
    let target = create_test_account_id_with(account_id);
    // Use random payload with account_id prefix to ensure unique transaction IDs
    let mut runner = TestRunner::default();
    let payload_strategy = prop::collection::vec(any::<u8>(), 10..100);
    let mut payload = payload_strategy.new_tree(&mut runner).unwrap().current();
    // Prepend account_id to make it deterministic per account
    payload.insert(0, account_id);
    OLMempoolTransaction::new_generic_account_message(target, payload, attachment)
        .expect("Should create transaction")
}

/// In-memory state provider for fast testing without database infrastructure.
///
/// Stores states in a `HashMap` for quick lookup. Thread-safe via `RwLock`.
#[derive(Debug)]
pub(crate) struct InMemoryStateProvider {
    states: RwLock<HashMap<OLBlockCommitment, Arc<OLState>>>,
}

impl InMemoryStateProvider {
    /// Create a provider with an initial state at the given tip.
    pub(crate) fn with_initial_state(tip: OLBlockCommitment, state: OLState) -> Self {
        let mut states = HashMap::new();
        states.insert(tip, Arc::new(state));
        Self {
            states: RwLock::new(states),
        }
    }

    /// Insert a state at the given tip (useful for test setup).
    pub(crate) fn insert_state(&self, tip: OLBlockCommitment, state: OLState) {
        let mut states = self.states.write().unwrap();
        states.insert(tip, Arc::new(state));
    }

    /// Retrieves the state for a given chain tip asynchronously.
    pub(crate) async fn get_state_for_tip_async(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<Arc<OLState>>, InMemoryStateProviderError> {
        let states = self
            .states
            .read()
            .map_err(|e| InMemoryStateProviderError::LockPoisoned(format!("{}", e)))?;
        Ok(states.get(&tip).cloned())
    }

    /// Retrieves the state for a given chain tip in a blocking manner.
    pub(crate) fn get_state_for_tip_blocking(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<Arc<OLState>>, InMemoryStateProviderError> {
        let states = self
            .states
            .read()
            .map_err(|e| InMemoryStateProviderError::LockPoisoned(format!("{}", e)))?;
        Ok(states.get(&tip).cloned())
    }
}

/// Error type for in-memory state provider (used in tests).
#[derive(Debug, thiserror::Error)]
pub(crate) enum InMemoryStateProviderError {
    #[error("lock poisoned: {0}")]
    LockPoisoned(String),
}

impl StateProvider for InMemoryStateProvider {
    type State = OLState;
    type Error = InMemoryStateProviderError;

    fn get_state_for_tip_async(
        &self,
        tip: OLBlockCommitment,
    ) -> impl Future<Output = Result<Option<Arc<Self::State>>, Self::Error>> + Send {
        self.get_state_for_tip_async(tip)
    }

    fn get_state_for_tip_blocking(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<Arc<Self::State>>, Self::Error> {
        self.get_state_for_tip_blocking(tip)
    }
}
