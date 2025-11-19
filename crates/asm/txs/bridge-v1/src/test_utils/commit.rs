use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    hashes::Hash,
    sighash::{Prevouts, SighashCache, TapSighashType},
};
use strata_crypto::{EvenSecretKey, test_utils::schnorr::create_musig2_signature};
use strata_l1_txfmt::{ParseConfig, TagData};

use crate::{
    commit::CommitInfo,
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, COMMIT_TX_TYPE},
    test_utils::{TEST_MAGIC_BYTES, create_tx_with_n_of_n_multisig_output},
};

/// Creates a commit transaction for testing purposes that spends from an N/N multisig.
///
/// This function creates two transactions:
/// 1. A funding transaction with an output locked to an N/N MuSig2 aggregated public key
/// 2. A commit transaction that spends from the funding tx with proper MuSig2 signature
///
/// The commit transaction follows the full SPS-50 specification and contains:
/// - Input: Spends from a P2TR N/N multisig output (from funding tx) with proper MuSig2 signature
/// - Output 0: OP_RETURN with full SPS-50 format: MAGIC + SUBPROTOCOL_ID + TX_TYPE + AUX_DATA
///
/// The transaction is fully compatible with the SPS-50 parser and can be parsed by `ParseConfig`.
///
/// # Parameters
///
/// - `commit_info` - The commit information specifying the deposit index being committed to
/// - `operators_privkeys` - Private keys of all operators (N/N multisig)
///
/// # Returns
///
/// A tuple of `(funding_tx, commit_tx)` where both transactions are ready for testing
pub fn create_test_commit_tx(
    commit_info: &CommitInfo,
    operators_privkeys: &[EvenSecretKey],
) -> (Transaction, Transaction) {
    // Create funding transaction with N/N multisig output (using dummy amount)
    let funding_tx = create_tx_with_n_of_n_multisig_output(operators_privkeys, Amount::ZERO);

    // Reference the funding tx output
    let funding_outpoint = OutPoint {
        txid: funding_tx.compute_txid(),
        vout: 0,
    };

    let prev_txout = funding_tx.output[0].clone();

    // Auxiliary data: [DEPOSIT_IDX][GAME_IDX]
    let mut aux_data = Vec::with_capacity(8);
    aux_data.extend_from_slice(&commit_info.deposit_idx.to_be_bytes());
    aux_data.extend_from_slice(&commit_info.game_idx.to_be_bytes());
    let td = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, COMMIT_TX_TYPE, aux_data).unwrap();
    let op_return_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&td.as_ref())
        .unwrap();

    // Build outputs - only OP_RETURN output
    let outputs = vec![
        // OP_RETURN output with SPS-50 tagged payload
        TxOut {
            value: Amount::from_sat(0),
            script_pubkey: op_return_script,
        },
    ];

    // Create unsigned transaction
    let unsigned_tx = Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: funding_outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: outputs.clone(),
    };

    // Compute sighash for taproot key-spend signature
    let prevtxouts = [prev_txout];
    let prevouts = Prevouts::All(&prevtxouts);
    let mut sighash_cache = SighashCache::new(&unsigned_tx);
    let sighash = sighash_cache
        .taproot_key_spend_signature_hash(0, &prevouts, TapSighashType::Default)
        .unwrap();

    let msg = sighash.to_byte_array();

    // Create MuSig2 signature using all operators (N/N)
    let final_signature = create_musig2_signature(operators_privkeys, &msg, None);

    // Create the final signed commit transaction
    let commit_tx = Transaction {
        version: unsigned_tx.version,
        lock_time: unsigned_tx.lock_time,
        input: vec![TxIn {
            previous_output: funding_outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[final_signature.serialize().as_slice()]),
        }],
        output: outputs,
    };

    (funding_tx, commit_tx)
}
