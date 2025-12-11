//! Withdrawal Fulfillment Transaction creation utilities for testing
//!
//! Provides both simple test utilities and comprehensive transaction builders for
//! withdrawal fulfillment transactions.

use bitcoin::Transaction;
use strata_l1_txfmt::ParseConfig;

use crate::{
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx},
    withdrawal_fulfillment::WithdrawalFulfillmentInfo,
};

/// Creates a withdrawal fulfillment transaction for testing purposes
pub fn create_test_withdrawal_fulfillment_tx(
    withdrawal_info: &WithdrawalFulfillmentInfo,
) -> Transaction {
    let mut tx = create_dummy_tx(1, 2);
    let td = withdrawal_info.header_aux().build_tag_data().unwrap();
    let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&td.as_ref())
        .unwrap();

    tx.output[0].script_pubkey = sps_50_script;
    tx.output[1].script_pubkey = withdrawal_info.withdrawal_destination().clone();
    tx.output[1].value = withdrawal_info.withdrawal_amount().into();

    tx
}
