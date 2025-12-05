//! Test utilities for mempool tests.

use proptest::{
    strategy::{Strategy, ValueTree},
    test_runner::TestRunner,
};
use strata_acct_types::AccountId;
use strata_ol_chain_types_new::{TransactionAttachment, test_utils as ol_test_utils};
use strata_snark_acct_types::SnarkAccountUpdate;

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
