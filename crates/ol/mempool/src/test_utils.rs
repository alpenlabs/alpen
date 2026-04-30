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
use strata_identifiers::{Buf32, Hash, L1BlockCommitment, OLBlockCommitment, OLBlockId, Slot};
use strata_ledger_types::{
    IAccountStateMut, ISnarkAccountStateMut, IStateAccessorMut, NewAccountData, NewAccountTypeState,
};
use strata_ol_chain_types_new::{
    ClaimList, OLTransaction, OLTransactionData, ProofSatisfierList, SauTxLedgerRefs,
    SauTxOperationData, SauTxPayload, SauTxProofState, SauTxUpdateData, TransactionPayload,
    TxConstraints, TxProofs, test_utils as ol_test_utils,
};
use strata_ol_params::OLParams;
use strata_ol_state_provider::StateProvider;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::OLState;
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{Seqno, SnarkAccountUpdate, UpdateOperationData};
use strata_storage::create_node_storage;
use threadpool::ThreadPool;

use crate::{state::MempoolContext, types::OLMempoolConfig};

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

/// Create test transaction constraints using proptest strategy.
pub(crate) fn create_test_constraints() -> TxConstraints {
    let mut runner = TestRunner::default();
    ol_test_utils::tx_constraints_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current()
}

/// Create a test snark account update (base_update only, no accumulator proofs).
pub(crate) fn create_test_snark_update() -> SnarkAccountUpdate {
    // Use ol-chain-types strategy to generate a SauTxPayload, then extract a SnarkAccountUpdate
    let mut runner = TestRunner::default();
    let sau_payload = ol_test_utils::sau_tx_payload_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();

    let operation = sau_payload.operation();
    let update_data = operation.update();
    let proof_state = strata_snark_acct_types::ProofState::new(
        update_data.proof_state().inner_state_root(),
        update_data.proof_state().new_next_msg_idx(),
    );
    let messages: Vec<_> = operation.messages_iter().cloned().collect();
    let ledger_refs = strata_snark_acct_types::LedgerRefs::new(
        operation
            .ledger_refs()
            .asm_history_proofs()
            .map(|c| c.claims.iter().cloned().collect())
            .unwrap_or_default(),
    );
    let snark_operation = UpdateOperationData::new(
        update_data.seq_no(),
        proof_state,
        messages,
        ledger_refs,
        strata_snark_acct_types::UpdateOutputs::new(vec![], vec![]),
        update_data.extra_data().to_vec(),
    );
    SnarkAccountUpdate::new(snark_operation, vec![])
}

/// Create test transaction constraints with optional min/max slots.
pub(crate) fn create_test_constraints_with_slots(
    min_slot: Option<Slot>,
    max_slot: Option<Slot>,
) -> TxConstraints {
    TxConstraints::new(min_slot, max_slot)
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

pub(crate) fn create_test_snark_tx_from_update(
    target: AccountId,
    base_update: SnarkAccountUpdate,
    constraints: TxConstraints,
) -> OLTransaction {
    let operation = base_update.operation();
    let proof_state = operation.new_proof_state();
    let sau_proof_state =
        SauTxProofState::new(proof_state.next_inbox_msg_idx(), proof_state.inner_state());
    let sau_update_data = SauTxUpdateData::new(
        operation.seq_no(),
        sau_proof_state,
        operation.extra_data().to_vec(),
    );

    let asm_hist_refs = operation.ledger_refs().l1_header_refs();
    let sau_ledger_refs = if asm_hist_refs.is_empty() {
        SauTxLedgerRefs::new_empty()
    } else {
        let claim_list =
            ClaimList::new(asm_hist_refs.to_vec()).expect("snark update has too many ASM claims");
        SauTxLedgerRefs::new_with_claims(claim_list)
    };

    let messages = operation.processed_messages().to_vec();
    let sau_operation_data = SauTxOperationData::new(sau_update_data, messages, sau_ledger_refs);
    let payload =
        TransactionPayload::SnarkAccountUpdate(SauTxPayload::new(target, sau_operation_data));
    let effects = operation.outputs().to_tx_effects();
    let data = OLTransactionData::new(payload, effects).with_constraints(constraints);
    let proofs = TxProofs::new(
        ProofSatisfierList::single(base_update.update_proof().to_vec()),
        None,
    );
    OLTransaction::new(data, proofs)
}

/// Returns the target account from a transaction for tests.
///
/// All current OL transaction payload variants include a target account.
pub(crate) fn tx_target(tx: &OLTransaction) -> AccountId {
    tx.target()
        .expect("all OLTransaction payload variants must have a target account")
}

/// Returns the snark sequence number for snark transactions.
///
/// Returns `None` for non-snark payload variants.
pub(crate) fn snark_seq_no(tx: &OLTransaction) -> Option<u64> {
    match tx.payload() {
        TransactionPayload::SnarkAccountUpdate(payload) => {
            Some(payload.operation().update().seq_no())
        }
        TransactionPayload::GenericAccountMessage(_) => None,
    }
}

/// Returns a transaction with updated constraints.
pub(crate) fn with_constraints(tx: OLTransaction, constraints: TxConstraints) -> OLTransaction {
    let data = tx.data().clone().with_constraints(constraints);
    OLTransaction::new(data, tx.proofs().clone())
}

/// Returns a transaction with updated `max_slot` in constraints.
pub(crate) fn with_max_slot(tx: OLTransaction, max_slot: Option<Slot>) -> OLTransaction {
    let mut constraints = tx.constraints().clone();
    constraints.set_max_slot(max_slot);
    with_constraints(tx, constraints)
}

/// Creates a genesis OLState using minimal empty parameters.
pub(crate) fn create_test_genesis_state() -> OLState {
    let params = OLParams::new_empty(L1BlockCommitment::default());
    OLState::from_genesis_params(&params).expect("valid params")
}

/// Create a test OLState with an empty account for the given account ID.
///
/// Returns a state with an empty account for the given account ID at the specified slot.
/// This allows generic account message transactions to pass account existence checks.
pub(crate) fn create_test_ol_state_with_account(
    account_id: AccountId,
    slot: u64,
) -> MemoryStateBaseLayer {
    let state = create_test_genesis_state();
    let mut layer = MemoryStateBaseLayer::new(state);
    layer.set_cur_slot(slot);
    // Create an empty account so it exists for validation
    let new_acct = NewAccountData::new(BitcoinAmount::from(0), NewAccountTypeState::Empty);
    layer.create_new_account(account_id, new_acct).unwrap();
    layer
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
) -> MemoryStateBaseLayer {
    let state = create_test_genesis_state();
    let mut layer = MemoryStateBaseLayer::new(state);
    layer.set_cur_slot(slot);
    // Create a fresh snark account, then update its sequence number
    let new_acct = NewAccountData::new(
        BitcoinAmount::from(0),
        NewAccountTypeState::Snark {
            update_vk: PredicateKey::always_accept(),
            initial_state_root: Hash::zero(),
        },
    );
    layer.create_new_account(account_id, new_acct).unwrap();

    // Update the sequence number using the mutable interface
    layer
        .update_account(account_id, |account| {
            let snark_account = account.as_snark_account_mut().unwrap();
            snark_account.set_proof_state_directly(Hash::zero(), 0, Seqno::from(seq_no));
        })
        .unwrap();

    layer
}

/// Create a test snark account update transaction.
pub(crate) fn create_test_snark_tx() -> OLTransaction {
    create_test_snark_tx_from_update(
        create_test_account_id_with(1),
        create_test_snark_update(),
        create_test_constraints(),
    )
}

/// Create a test generic account message transaction.
/// Uses randomly generated constraints for tests where slot bounds are not fixed.
pub(crate) fn create_test_generic_tx() -> OLTransaction {
    let constraints = create_test_constraints();
    create_test_generic_tx_with_constraints(constraints)
}

/// Create a test generic account message transaction with constraints.
pub(crate) fn create_test_generic_tx_with_constraints(constraints: TxConstraints) -> OLTransaction {
    let target = create_test_account_id();
    let mut runner = TestRunner::default();
    let payload_strategy = prop::collection::vec(any::<u8>(), 10..100);
    let payload = payload_strategy.new_tree(&mut runner).unwrap().current();
    let data = OLTransactionData::new_gam(target, payload).with_constraints(constraints);
    OLTransaction::new(data, TxProofs::new_empty())
}

/// Create a test generic account message transaction with specific slot bounds.
pub(crate) fn create_test_generic_tx_with_slots(
    min_slot: Option<Slot>,
    max_slot: Option<Slot>,
) -> OLTransaction {
    let constraints = create_test_constraints_with_slots(min_slot, max_slot);
    create_test_generic_tx_with_constraints(constraints)
}

/// Create a test generic account message transaction with a specific payload size.
pub(crate) fn create_test_generic_tx_with_size(
    target: AccountId,
    size: usize,
    constraints: TxConstraints,
) -> OLTransaction {
    let mut runner = TestRunner::default();
    let payload_strategy = prop::collection::vec(any::<u8>(), size..=size);
    let payload = payload_strategy.new_tree(&mut runner).unwrap().current();
    let data = OLTransactionData::new_gam(target, payload).with_constraints(constraints);
    OLTransaction::new(data, TxProofs::new_empty())
}

/// Create a test transaction with a specific target account ID.
/// Uses the ID byte to create different account IDs, but the update content is randomly generated.
/// Uses an attachment without slot restrictions (min_slot=None, max_slot=None).
pub(crate) fn create_test_tx_with_id(id: u8) -> OLTransaction {
    let constraints = create_test_constraints_with_slots(None, None);
    create_test_snark_tx_from_update(
        create_test_account_id_with(id),
        create_test_snark_update(),
        constraints,
    )
}

/// Create a test snark transaction with a specific seq_no for deterministic ordering tests.
pub(crate) fn create_test_snark_tx_with_seq_no(account_id: u8, seq_no: u64) -> OLTransaction {
    create_test_snark_tx_with_seq_no_and_slots(account_id, seq_no, None, None)
}

/// Create a test snark transaction with a specific seq_no and slot bounds.
pub(crate) fn create_test_snark_tx_with_seq_no_and_slots(
    account_id: u8,
    seq_no: u64,
    min_slot: Option<Slot>,
    max_slot: Option<Slot>,
) -> OLTransaction {
    let mut runner = TestRunner::default();

    // Use constraints with specified slot bounds
    let constraints = create_test_constraints_with_slots(min_slot, max_slot);

    let sau_payload = ol_test_utils::sau_tx_payload_strategy()
        .new_tree(&mut runner)
        .unwrap()
        .current();

    let operation_data = sau_payload.operation();
    let update_data = operation_data.update();
    let proof_state = strata_snark_acct_types::ProofState::new(
        update_data.proof_state().inner_state_root(),
        update_data.proof_state().new_next_msg_idx(),
    );
    let messages: Vec<_> = operation_data.messages_iter().cloned().collect();
    let ledger_refs = strata_snark_acct_types::LedgerRefs::new(
        operation_data
            .ledger_refs()
            .asm_history_proofs()
            .map(|c| c.claims.iter().cloned().collect())
            .unwrap_or_default(),
    );

    let operation = UpdateOperationData::new(
        seq_no,
        proof_state,
        messages,
        ledger_refs,
        strata_snark_acct_types::UpdateOutputs::new(vec![], vec![]),
        update_data.extra_data().to_vec(),
    );

    let update = SnarkAccountUpdate::new(operation, vec![]);

    create_test_snark_tx_from_update(create_test_account_id_with(account_id), update, constraints)
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

    MempoolContext::new(config, test_storage, provider)
}

/// Create an InMemoryStateProvider with initial test state at the given tip.
///
/// Creates a genesis state with Snark accounts for test account IDs (0-255).
pub(crate) fn create_test_state_provider(tip: OLBlockCommitment) -> InMemoryStateProvider {
    let state = create_test_ol_state_for_tip(tip.slot());
    InMemoryStateProvider::from_initial_state(tip, state)
}

/// Create a test OL state at a given slot with Snark accounts.
///
/// Creates a genesis state with Snark accounts for test account IDs (0-255).
pub(crate) fn create_test_ol_state_for_tip(slot: u64) -> OLState {
    let state = create_test_genesis_state();
    let mut layer = MemoryStateBaseLayer::new(state);
    layer.set_cur_slot(slot);

    // Create Snark accounts for common test account IDs (0-255)
    for id_byte in 0..=255u8 {
        let account_id = create_test_account_id_with(id_byte);
        let new_acct = NewAccountData::new(
            BitcoinAmount::from(0),
            NewAccountTypeState::Snark {
                update_vk: PredicateKey::always_accept(),
                initial_state_root: Hash::zero(),
            },
        );
        if layer.create_new_account(account_id, new_acct).is_ok() {
            let _ = layer.update_account(account_id, |account| {
                let snark_account = account.as_snark_account_mut().unwrap();
                snark_account.set_proof_state_directly(Hash::zero(), 0, Seqno::from(0));
            });
        }
    }

    layer.into_inner()
}

/// Create a test generic account message transaction for a specific account.
/// Uses an attachment without slot restrictions (min_slot=None, max_slot=None).
/// Uses a unique random payload to ensure unique transaction IDs.
pub(crate) fn create_test_generic_tx_for_account(account_id: u8) -> OLTransaction {
    let constraints = create_test_constraints_with_slots(None, None);
    let target = create_test_account_id_with(account_id);
    // Use random payload with account_id prefix to ensure unique transaction IDs
    let mut runner = TestRunner::default();
    let payload_strategy = prop::collection::vec(any::<u8>(), 10..100);
    let mut payload = payload_strategy.new_tree(&mut runner).unwrap().current();
    // Prepend account_id to make it deterministic per account
    payload.insert(0, account_id);
    let data = OLTransactionData::new_gam(target, payload).with_constraints(constraints);
    OLTransaction::new(data, TxProofs::new_empty())
}

/// In-memory state provider for fast testing without database infrastructure.
///
/// Stores states in a `HashMap` for quick lookup. Thread-safe via `RwLock`.
#[derive(Debug)]
pub(crate) struct InMemoryStateProvider {
    states: RwLock<HashMap<OLBlockCommitment, OLState>>,
}

impl InMemoryStateProvider {
    /// Create a provider with an initial state at the given tip.
    pub(crate) fn from_initial_state(tip: OLBlockCommitment, state: OLState) -> Self {
        let mut states = HashMap::new();
        states.insert(tip, state);
        Self {
            states: RwLock::new(states),
        }
    }

    /// Insert a state at the given tip (useful for test setup).
    pub(crate) fn insert_state(&self, tip: OLBlockCommitment, state: OLState) {
        let mut states = self.states.write().unwrap();
        states.insert(tip, state);
    }

    /// Retrieves the state for a given chain tip asynchronously.
    pub(crate) async fn get_state_for_tip_async_inner(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<OLState>, InMemoryStateProviderError> {
        let states = self
            .states
            .read()
            .map_err(|e| InMemoryStateProviderError::LockPoisoned(format!("{}", e)))?;
        Ok(states.get(&tip).cloned())
    }

    /// Retrieves the state for a given chain tip in a blocking manner.
    pub(crate) fn get_state_for_tip_blocking_inner(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<OLState>, InMemoryStateProviderError> {
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

#[expect(clippy::manual_async_fn, reason = "forced by trait")]
impl StateProvider for InMemoryStateProvider {
    type State = MemoryStateBaseLayer;
    type Error = InMemoryStateProviderError;

    fn get_state_for_tip_async(
        &self,
        tip: OLBlockCommitment,
    ) -> impl Future<Output = Result<Option<Self::State>, Self::Error>> + Send {
        async move {
            Ok(self
                .get_state_for_tip_async_inner(tip)
                .await?
                .map(MemoryStateBaseLayer::new))
        }
    }

    fn get_state_for_tip_blocking(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<Self::State>, Self::Error> {
        Ok(self
            .get_state_for_tip_blocking_inner(tip)?
            .map(MemoryStateBaseLayer::new))
    }
}
