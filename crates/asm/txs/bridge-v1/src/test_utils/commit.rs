use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, absolute::LockTime,
    script::PushBytesBuf, transaction::Version,
};

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
    // Create SPS-50 tagged payload: [MAGIC][SUBPROTOCOL_ID][TX_TYPE][AUX_DATA]
    let mut tagged_payload = Vec::new();
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES); // 4 bytes magic
    tagged_payload.push(BRIDGE_V1_SUBPROTOCOL_ID); // 1 byte subprotocol ID
    tagged_payload.push(COMMIT_TX_TYPE); // 1 byte transaction type

    // Auxiliary data: [DEPOSIT_IDX]
    tagged_payload.extend_from_slice(&commit_info.deposit_idx.to_be_bytes()); // 4 bytes

    // Create OP_RETURN script with the tagged payload
    let op_return_script = ScriptBuf::new_op_return(
        PushBytesBuf::try_from(tagged_payload).expect("Tagged payload should fit in push bytes"),
    );

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
