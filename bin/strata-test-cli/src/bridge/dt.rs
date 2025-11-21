//! Handles Deposit Transaction (DT) creation.
//!
//! The CLI is responsible for signature aggregation and transaction signing.
//! All transaction structure and OP_RETURN construction is handled by asm/txs/bridge-v1.

use bdk_wallet::bitcoin::{
    Amount, Psbt, ScriptBuf, TapNodeHash, TapSighashType, Transaction, TxOut, Witness,
    consensus::deserialize,
    hashes::Hash,
    opcodes::all::{OP_CHECKSIGVERIFY, OP_CSV},
    script::Builder,
    sighash::{Prevouts, SighashCache},
    taproot::LeafVersion,
};
use secp256k1::SECP256K1;
use strata_asm_txs_bridge_v1::{
    deposit_request::parse_drt_from_tx,
    test_utils::{build_deposit_transaction, create_deposit_op_return},
};
use strata_crypto::{EvenSecretKey, test_utils::schnorr::create_musig2_signature};
use strata_primitives::{buf::Buf32, constants::RECOVER_DELAY};

use crate::{
    constants::{BRIDGE_OUT_AMOUNT, MAGIC_BYTES, NETWORK},
    error::Error,
    parse::{generate_taproot_address, parse_operator_keys},
};

/// Builds the timelock script for takeback functionality
fn build_timelock_script(recovery_pubkey_bytes: &[u8; 32]) -> ScriptBuf {
    Builder::new()
        .push_slice(recovery_pubkey_bytes)
        .push_opcode(OP_CHECKSIGVERIFY)
        .push_int(RECOVER_DELAY as i64)
        .push_opcode(OP_CSV)
        .into_script()
}

/// Creates a deposit transaction (DT)
///
/// # Arguments
/// * `tx_bytes` - Raw DRT transaction bytes
/// * `operator_keys` - Vector of operator secret keys as bytes (78 bytes each)
/// * `dt_index` - Deposit transaction index for metadata
///
/// # Returns
/// * `Result<Vec<u8>, Error>` - The signed and serialized deposit transaction
pub(crate) fn create_deposit_transaction_cli(
    tx_bytes: Vec<u8>,
    operator_keys: Vec<[u8; 78]>,
    dt_index: u32,
) -> Result<Vec<u8>, Error> {
    let drt_tx = deserialize(&tx_bytes)
        .map_err(|e| Error::TxParser(format!("Failed to parse DRT: {e}")))?;

    let signers = parse_operator_keys(&operator_keys)
        .map_err(|e| Error::TxBuilder(format!("Failed to parse operator keys: {e}")))?;

    let pubkeys = signers
        .iter()
        .map(|kp| Buf32::from(kp.x_only_public_key(SECP256K1).0))
        .collect::<Vec<_>>();

    let (_address, agg_pubkey) = generate_taproot_address(&pubkeys, NETWORK)
        .map_err(|e| Error::TxBuilder(e.to_string()))?;

    let drt_data = parse_drt_from_tx(&drt_tx, MAGIC_BYTES)
        .map_err(|e| Error::TxParser(format!("Failed to parse DRT: {}", e)))?;

    let takeback_script = build_timelock_script(&drt_data.take_back_leaf_hash);
    let takeback_hash = TapNodeHash::from_script(&takeback_script, LeafVersion::TapScript);

    // Use canonical OP_RETURN construction from asm/txs/bridge-v1
    let op_return_script = create_deposit_op_return(
        *MAGIC_BYTES,
        dt_index,
        takeback_hash,
        &drt_data.address,
    )
    .map_err(|e| Error::TxBuilder(e))?;

    // Build deposit transaction using canonical builder
    let unsigned_tx = build_deposit_transaction(
        drt_tx.compute_txid(),
        op_return_script,
        agg_pubkey,
        BRIDGE_OUT_AMOUNT,
    );

    // Find the P2TR output (the one that's not OP_RETURN)
    let deposit_request_output = drt_tx
        .output
        .iter()
        .find(|out| !out.script_pubkey.is_op_return())
        .ok_or_else(|| Error::TxParser("DRT has no P2TR output".to_string()))?;

    let prevout = TxOut {
        script_pubkey: deposit_request_output.script_pubkey.clone(),
        value: Amount::from_sat(drt_data.amt),
    };

    let signed_tx = sign_deposit_transaction(unsigned_tx, &prevout, takeback_hash, &signers)?;

    Ok(bdk_wallet::bitcoin::consensus::serialize(&signed_tx))
}

/// Signs a deposit transaction using MuSig2 aggregated signature.
///
/// Creates a PSBT from the unsigned transaction, computes the taproot key-spend
/// sighash, and generates a MuSig2 aggregated Schnorr signature from multiple
/// operator private keys. The signature is tweaked with the takeback script hash
/// to commit to the script path spend option.
///
/// # Arguments
/// * `unsigned_tx` - The unsigned deposit transaction to sign
/// * `prevout` - The DRT output being spent (contains script and amount)
/// * `takeback_hash` - Taproot hash of the takeback script for tweaking the signature
/// * `signers` - Array of operator private keys for MuSig2 aggregation
///
/// # Returns
/// Fully signed transaction ready for broadcast
fn sign_deposit_transaction(
    unsigned_tx: Transaction,
    prevout: &TxOut,
    takeback_hash: TapNodeHash,
    signers: &[EvenSecretKey],
) -> Result<Transaction, Error> {
    let mut psbt = Psbt::from_unsigned_tx(unsigned_tx.clone())
        .map_err(|e| Error::TxBuilder(format!("Failed to create PSBT: {}", e)))?;

    if let Some(input) = psbt.inputs.get_mut(0) {
        input.witness_utxo = Some(prevout.clone());
        input.sighash_type = Some(TapSighashType::Default.into());
    }

    let prevouts_ref = Prevouts::All(std::slice::from_ref(prevout));
    let mut sighash_cache = SighashCache::new(&unsigned_tx);

    let sighash = sighash_cache
        .taproot_key_spend_signature_hash(0, &prevouts_ref, TapSighashType::Default)
        .map_err(|e| Error::TxBuilder(format!("Sighash creation failed: {e}")))?;

    let msg = sighash.to_byte_array();
    let tweak_bytes = Some(takeback_hash.to_byte_array());
    let schnorr_sig = create_musig2_signature(signers, &msg, tweak_bytes);

    let signature = bdk_wallet::bitcoin::taproot::Signature {
        signature: schnorr_sig.into(),
        sighash_type: TapSighashType::Default,
    };

    if let Some(input) = psbt.inputs.get_mut(0) {
        input.tap_key_sig = Some(signature);
    }

    finalize_and_extract_tx(psbt)
}

/// Finalizes a PSBT by converting signatures to witness data and extracts the transaction.
///
/// Takes a PSBT with taproot key-spend signatures and converts them into the
/// final witness format required for broadcast. The witness for a taproot key-spend
/// contains only the signature (no script or other data).
///
/// # Arguments
/// * `psbt` - PSBT with `tap_key_sig` populated for each input
///
/// # Returns
/// Finalized transaction ready for broadcast
fn finalize_and_extract_tx(mut psbt: Psbt) -> Result<Transaction, Error> {
    for input in &mut psbt.inputs {
        if input.tap_key_sig.is_some() {
            input.final_script_witness = Some(Witness::new());
            if let Some(sig) = &input.tap_key_sig {
                input
                    .final_script_witness
                    .as_mut()
                    .unwrap()
                    .push(sig.to_vec());
            }
        }
    }

    psbt.clone()
        .extract_tx()
        .map_err(|e| Error::TxBuilder(format!("Transaction extraction failed: {}", e)))
}
