//! Test utilities for mempool tests.

use proptest::{
    strategy::{Strategy, ValueTree},
    test_runner::TestRunner,
};
use strata_acct_types::AccountId;
use strata_ol_chain_types_new::{TransactionAttachment, test_utils as ol_test_utils};
use strata_snark_acct_types::SnarkAccountUpdate;

use crate::types::{OLMempoolSnarkAcctUpdateTxPayload, OLMempoolTransaction, OLMempoolTxPayload};

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

/// Create a test transaction attachment with optional min/max slots.
pub(crate) fn create_test_attachment_with_slots(
    min_slot: Option<u64>,
    max_slot: Option<u64>,
) -> TransactionAttachment {
    TransactionAttachment::new(min_slot, max_slot)
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

/// Create a test snark account update transaction.
pub(crate) fn create_test_snark_tx() -> OLMempoolTransaction {
    create_test_mempool_tx(create_test_snark_payload())
}

/// Create a test generic account message transaction.
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
