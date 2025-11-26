use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, absolute::LockTime,
    transaction::Version,
};
use strata_codec::encode_to_vec;
use strata_l1_txfmt::{ParseConfig, TagData};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, WITHDRAWAL_FULFILLMENT_TX_TYPE},
    test_utils::TEST_MAGIC_BYTES,
    withdrawal_fulfillment::WithdrawalFulfillmentInfo,
};

/// Creates a withdrawal fulfillment transaction for testing purposes.
///
/// This function constructs a Bitcoin transaction that follows the full SPS-50 specification
/// for withdrawal fulfillment transactions. The transaction contains:
/// - Input: A dummy input spending from a previous output
/// - Output 0: OP_RETURN with full SPS-50 format: MAGIC + SUBPROTOCOL_ID + TX_TYPE + AUX_DATA
/// - Output 1: The actual withdrawal payment to the recipient address
///
/// The transaction is fully compatible with the SPS-50 parser and can be parsed by `ParseConfig`.
///
/// # Parameters
///
/// - `withdrawal_info` - The withdrawal information specifying operator, deposit details, recipient
///   address, and withdrawal amount
///
/// # Returns
///
/// A [`Transaction`] that follows the SPS-50 specification and can be parsed for testing.
pub fn create_test_withdrawal_fulfillment_tx(
    withdrawal_info: &WithdrawalFulfillmentInfo,
) -> Transaction {
    let aux_data = encode_to_vec(&withdrawal_info.header_aux).unwrap();
    let td = TagData::new(
        BRIDGE_V1_SUBPROTOCOL_ID,
        WITHDRAWAL_FULFILLMENT_TX_TYPE,
        aux_data,
    )
    .unwrap();
    let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&td.as_ref())
        .unwrap();

    Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(), // Dummy input
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[vec![0u8; 64]]), // Dummy witness
        }],
        output: vec![
            // OP_RETURN output with SPS-50 tagged payload
            TxOut {
                value: Amount::from_sat(0),
                script_pubkey: sps_50_script,
            },
            // Withdrawal fulfillment output
            TxOut {
                value: Amount::from_sat(withdrawal_info.withdrawal_amount.to_sat()),
                script_pubkey: withdrawal_info.withdrawal_destination.clone(),
            },
        ],
    }
}
