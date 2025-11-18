use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, absolute::LockTime,
    transaction::Version,
};
use strata_l1_txfmt::{ParseConfig, TagData};

use crate::{
    commit::CommitInfo,
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, COMMIT_TX_TYPE},
    test_utils::TEST_MAGIC_BYTES,
};

/// Creates a commit transaction for testing purposes.
///
/// This function constructs a Bitcoin transaction that follows the full SPS-50 specification
/// for commit transactions. The transaction contains:
/// - Input: A dummy input spending from a previous output
/// - Output 0: OP_RETURN with full SPS-50 format: MAGIC + SUBPROTOCOL_ID + TX_TYPE + AUX_DATA
///
/// The transaction is fully compatible with the SPS-50 parser and can be parsed by `ParseConfig`.
///
/// # Parameters
///
/// - `commit_info` - The commit information specifying the deposit index being committed to
///
/// # Returns
///
/// A [`Transaction`] that follows the SPS-50 specification and can be parsed for testing.
pub fn create_test_commit_tx(commit_info: &CommitInfo) -> Transaction {
    // Auxiliary data: [DEPOSIT_IDX]
    let aux_data = commit_info.deposit_idx.to_be_bytes();
    let td = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, COMMIT_TX_TYPE, aux_data.to_vec()).unwrap();
    let op_return_script = ParseConfig::new(*TEST_MAGIC_BYTES)
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
                script_pubkey: op_return_script,
            },
        ],
    }
}
