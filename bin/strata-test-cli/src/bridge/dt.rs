//! Handles Deposit Transaction (DT) creation.
//!
//! The CLI is responsible for signature aggregation and transaction signing.
//! All transaction structure and OP_RETURN construction is handled by asm/txs/bridge-v1.

use std::slice;

use bdk_wallet::bitcoin::{
    consensus::{self, deserialize},
    hashes::Hash,
    sighash::{Prevouts, SighashCache},
    taproot, OutPoint, Psbt, ScriptBuf, TapNodeHash, TapSighashType, Transaction, TxOut, Witness,
};
use secp256k1::SECP256K1;
use strata_asm_txs_bridge_v1::{
    deposit::DepositTxHeaderAux,
    deposit_request::{build_deposit_request_spend_info, parse_drt},
    test_utils::create_dummy_tx,
};
use strata_crypto::{
    test_utils::schnorr::{create_musig2_signature, Musig2Tweak},
    EvenSecretKey,
};
use strata_l1_txfmt::ParseConfig;
use strata_primitives::{buf::Buf32, constants::RECOVER_DELAY};

use crate::{
    constants::{BRIDGE_OUT_AMOUNT, MAGIC_BYTES, NETWORK},
    error::Error,
    parse::{generate_taproot_address, parse_operator_keys},
};

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
    let drt_tx =
        deserialize(&tx_bytes).map_err(|e| Error::TxParser(format!("Failed to parse DRT: {e}")))?;

    let signers = parse_operator_keys(&operator_keys)
        .map_err(|e| Error::TxBuilder(format!("Failed to parse operator keys: {e}")))?;

    let pubkeys = signers
        .iter()
        .map(|kp| Buf32::from(kp.x_only_public_key(SECP256K1).0.serialize()))
        .collect::<Vec<_>>();

    let (_address, agg_pubkey) =
        generate_taproot_address(&pubkeys, NETWORK).map_err(|e| Error::TxBuilder(e.to_string()))?;

    let drt_data =
        parse_drt(&drt_tx).map_err(|e| Error::TxParser(format!("Failed to parse DRT: {}", e)))?;

    let takeback_hash = build_deposit_request_spend_info(
        drt_data.header_aux().recovery_pk(),
        agg_pubkey,
        RECOVER_DELAY,
    )
    .merkle_root()
    .ok_or_else(|| Error::TxBuilder("Missing takeback script merkle root".to_string()))?;

    // Use canonical OP_RETURN construction from asm/txs/bridge-v1
    let dt_tag = DepositTxHeaderAux::new(dt_index).build_tag_data();
    let sps50_script = ParseConfig::new(MAGIC_BYTES)
        .encode_script_buf(&dt_tag.as_ref())
        .map_err(|e| Error::TxBuilder(e.to_string()))?;

    let mut unsigned_tx = create_dummy_tx(1, 2);
    unsigned_tx.output[0].script_pubkey = sps50_script;
    unsigned_tx.output[1].value = BRIDGE_OUT_AMOUNT;
    unsigned_tx.output[1].script_pubkey = ScriptBuf::new_p2tr(SECP256K1, agg_pubkey, None);

    unsigned_tx.input[0].previous_output = OutPoint::new(drt_tx.compute_txid(), 1);

    // Per spec: P2TR deposit request output is at index 1
    let deposit_request_output = drt_tx
        .output
        .get(1)
        .ok_or_else(|| Error::TxParser("DRT missing P2TR output at index 1".to_string()))?;

    let signed_tx =
        sign_deposit_transaction(unsigned_tx, deposit_request_output, takeback_hash, &signers)?;

    Ok(consensus::serialize(&signed_tx))
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

    let prevouts_ref = Prevouts::All(slice::from_ref(prevout));
    let mut sighash_cache = SighashCache::new(&unsigned_tx);

    let sighash = sighash_cache
        .taproot_key_spend_signature_hash(0, &prevouts_ref, TapSighashType::Default)
        .map_err(|e| Error::TxBuilder(format!("Sighash creation failed: {e}")))?;

    let msg = sighash.to_byte_array();
    let tweak = Musig2Tweak::TaprootScript(takeback_hash.to_byte_array());
    let schnorr_sig = create_musig2_signature(signers, &msg, tweak);

    let signature = taproot::Signature {
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
