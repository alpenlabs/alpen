use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, absolute::LockTime,
    transaction::Version,
};
use strata_codec::encode_to_vec;
use strata_l1_txfmt::{ParseConfig, TagData};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, SLASH_TX_TYPE},
    slash::SlashInfo,
    test_utils::TEST_MAGIC_BYTES,
};

/// Creates a slash transaction for testing purposes.
///
/// Builds a Bitcoin transaction that follows the SPS-50 slash transaction format with:
/// - Two inputs (contest connector at index 0, stake connector at index 1)
/// - OP_RETURN output (index 0) carrying `[MAGIC][SUBPROTOCOL_ID][TX_TYPE][AUX_DATA]`
/// - A dummy payout output
pub fn create_test_slash_tx(info: &SlashInfo) -> Transaction {
    // Encode auxiliary data and construct op_return script
    let aux_data = encode_to_vec(&info.header_aux).unwrap();
    let tag_data = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, SLASH_TX_TYPE, aux_data).unwrap();
    let op_return_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&tag_data.as_ref())
        .unwrap();

    Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![
            TxIn {
                previous_output: OutPoint::null(), // contest connector (unused in parser)
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::from_slice(&[vec![0u8; 64]]),
            },
            TxIn {
                previous_output: info.second_input_outpoint.0, // stake connector we validate
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::from_slice(&[vec![0u8; 64]]),
            },
        ],
        output: vec![
            TxOut {
                value: Amount::from_sat(0),
                script_pubkey: op_return_script,
            },
            // Dummy payout/change output
            TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: ScriptBuf::new(),
            },
        ],
    }
}
