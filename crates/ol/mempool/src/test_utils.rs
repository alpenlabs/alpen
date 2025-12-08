//! Test utilities for mempool tests.

use proptest::{
    strategy::{Strategy, ValueTree},
    test_runner::TestRunner,
};
use strata_acct_types::{AccountId, BitcoinAmount, Hash};
use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
use strata_ledger_types::{
    AccountTypeState as LedgerAccountTypeState, IAccountStateMut, ISnarkAccountStateMut,
    IStateAccessor, NewAccountData,
};
use strata_ol_chain_types_new::{TransactionAttachment, test_utils as ol_test_utils};
use strata_ol_state_types::{NativeSnarkAccountState, OLState};
use strata_snark_acct_types::{Seqno, SnarkAccountUpdate};

use crate::{OLMempoolTransaction, OLMempoolTxPayload};

/// Create a test account ID using proptest strategy.
pub(crate) fn create_test_account_id() -> AccountId {
    let mut runner = TestRunner::default();
    proptest::arbitrary::any::<[u8; 32]>()
        .new_tree(&mut runner)
        .unwrap()
        .current()
        .into()
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

/// Create a test transaction attachment with specific slot bounds.
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
    use crate::OLMempoolSnarkAcctUpdateTxPayload;

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

/// Create a test mempool transaction with the specified payload.
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
            .expect("Failed to create generic account message transaction")
        }
    }
}

/// Create a test OLState with an empty account for the given account ID.
///
/// Returns a genesis state with an empty account for the given account ID.
/// This allows generic account message transactions to pass account existence checks.
pub(crate) fn create_test_ol_state_with_account(account_id: AccountId) -> OLState {
    let mut state = OLState::new_genesis();
    // Create an empty account so it exists for validation
    let new_acct = NewAccountData::new(BitcoinAmount::from(0), LedgerAccountTypeState::Empty);
    state.create_new_account(account_id, new_acct).unwrap();
    state
}

/// Create a test OLState with a Snark account for testing SnarkAccountUpdate transactions.
///
/// # Arguments
/// * `account_id` - The account ID to create
/// * `seq_no` - The initial sequence number for the Snark account
///
/// # Returns
/// An `OLState` with the specified Snark account
pub(crate) fn create_test_ol_state_with_snark_account(
    account_id: AccountId,
    seq_no: u64,
) -> OLState {
    let mut state = OLState::new_genesis();
    // Create a fresh snark account, then update its sequence number
    let snark_state = NativeSnarkAccountState::new_fresh(Hash::from([0u8; 32]));
    let new_acct = NewAccountData::new(
        BitcoinAmount::from(0),
        LedgerAccountTypeState::Snark(snark_state),
    );
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
pub(crate) fn create_test_generic_tx() -> OLMempoolTransaction {
    create_test_mempool_tx(create_test_generic_payload())
}

/// Create a test generic account message transaction.
pub(crate) fn create_test_generic_tx_with_attachment(
    attachment: TransactionAttachment,
) -> OLMempoolTransaction {
    let target = create_test_account_id();
    let payload = vec![1, 2, 3];
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
    size: usize,
    attachment: TransactionAttachment,
) -> OLMempoolTransaction {
    let target = create_test_account_id();
    let payload = vec![0u8; size];
    OLMempoolTransaction::new_generic_account_message(target, payload, attachment)
        .expect("Should create transaction")
}
