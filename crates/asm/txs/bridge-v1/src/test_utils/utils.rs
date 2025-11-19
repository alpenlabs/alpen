use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    absolute::LockTime, script::PushBytesBuf, secp256k1::Secp256k1, transaction::Version,
};
use strata_asm_common::TxInputRef;
use strata_crypto::{EvenSecretKey, test_utils::schnorr::create_agg_pubkey_from_privkeys};
use strata_l1_txfmt::ParseConfig;

use crate::test_utils::TEST_MAGIC_BYTES;

// Helper function to create tagged payload with custom parameters
pub fn create_tagged_payload(subprotocol_id: u8, tx_type: u8, aux_data: Vec<u8>) -> Vec<u8> {
    let mut tagged_payload = Vec::new();
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES);
    tagged_payload.push(subprotocol_id); // 1 byte subprotocol ID
    tagged_payload.push(tx_type); // 1 byte transaction type
    tagged_payload.extend_from_slice(&aux_data);
    tagged_payload
}

// Helper function to mutate transaction OP_RETURN output
pub fn mutate_op_return_output(tx: &mut Transaction, tagged_payload: Vec<u8>) {
    tx.output[0].script_pubkey =
        ScriptBuf::new_op_return(PushBytesBuf::try_from(tagged_payload).unwrap());
}

// Helper function to parse transaction
pub fn parse_tx(tx: &Transaction) -> TxInputRef<'_> {
    let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
    let tag_data = parser.try_parse_tx(tx).expect("Should parse transaction");
    TxInputRef::new(tx, tag_data)
}

/// Creates a transaction with a UTXO locked to an N/N multisig for testing.
///
/// This creates a transaction with a dummy input and an output locked to an N/N MuSig2
/// aggregated public key. The resulting transaction can be used as a funding source for
/// other test transactions that need to spend from a multisig UTXO.
///
/// # Arguments
///
/// - `operators_privkeys` - Private keys of all operators (N/N multisig)
/// - `amount` - Amount to lock in the multisig output
///
/// # Returns
///
/// A [`Transaction`] with a P2TR output locked to the aggregated key and a dummy input
pub fn create_tx_with_n_of_n_multisig_output(
    operators_privkeys: &[EvenSecretKey],
    amount: Amount,
) -> Transaction {
    let secp = Secp256k1::new();
    let aggregated_xonly = create_agg_pubkey_from_privkeys(operators_privkeys);
    let multisig_script = ScriptBuf::new_p2tr(&secp, aggregated_xonly, None);

    Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(), // Dummy input
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[vec![0u8; 64]]), // Dummy witness
        }],
        output: vec![TxOut {
            value: amount,
            script_pubkey: multisig_script,
        }],
    }
}
