use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, absolute::LockTime,
    transaction::Version,
};
use strata_asm_common::TxInputRef;
use strata_l1_txfmt::{ParseConfig, TagData};

use crate::test_utils::TEST_MAGIC_BYTES;

// Helper function to mutate SPS 50 transaction auxiliary data
pub fn mutate_aux_data(tx: &mut Transaction, new_aux: Vec<u8>) {
    let config = ParseConfig::new(*TEST_MAGIC_BYTES);
    let td = config.try_parse_tx(tx).unwrap();
    let new_td = TagData::new(td.subproto_id(), td.tx_type(), new_aux).unwrap();
    let new_scriptbuf = config.encode_script_buf(&new_td.as_ref()).unwrap();
    tx.output[0].script_pubkey = new_scriptbuf
}

// Helper function to parse transaction
pub fn parse_tx(tx: &Transaction) -> TxInputRef<'_> {
    let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
    let tag_data = parser.try_parse_tx(tx).expect("Should parse transaction");
    TxInputRef::new(tx, tag_data)
}

/// Creates a dummy Bitcoin transaction with the specified number of inputs and outputs.
///
/// The inputs will have null previous outputs and empty script sigs.
/// The outputs will have zero value and empty script pubkeys.
/// The transaction version is set to 2, and lock time to 0.
pub fn create_dummy_tx(num_inputs: usize, num_outputs: usize) -> Transaction {
    let input = (0..num_inputs)
        .map(|_| TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[vec![0u8; 64]]),
        })
        .collect();

    let output = (0..num_outputs)
        .map(|_| TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new(),
        })
        .collect();

    Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input,
        output,
    }
}
