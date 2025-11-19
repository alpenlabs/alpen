//! Deposit Transaction (DT) creation with DRT parsing and signing

use bdk_wallet::bitcoin::{
    Amount, Psbt, TapNodeHash, TapSighashType, Transaction, TxOut, Witness,
    consensus::deserialize,
    hashes::Hash,
    taproot::LeafVersion,
};
use secp256k1::SECP256K1;
use strata_asm_txs_bridge_v1::test_utils::{
    build_deposit_transaction, build_timelock_script,
};
use strata_crypto::{EvenSecretKey, test_utils::schnorr::create_musig2_signature};
use strata_primitives::buf::Buf32;

use crate::{
    constants::{BRIDGE_OUT_AMOUNT, MAGIC_BYTES, NETWORK},
    error::Error,
    parse::{generate_taproot_address, parse_drt, parse_operator_keys},
};

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

    let (address, agg_pubkey) = generate_taproot_address(&pubkeys, NETWORK)
        .map_err(|e| Error::TxBuilder(e.to_string()))?;

    let drt_data = parse_drt(&drt_tx, address, agg_pubkey)?;

    let takeback_script = build_timelock_script(&drt_data.take_back_leaf_hash)
        .map_err(|e| Error::TxBuilder(format!("Failed to build takeback script: {e}")))?;
    let takeback_hash = TapNodeHash::from_script(&takeback_script, LeafVersion::TapScript);

    let unsigned_tx = build_deposit_transaction(
        drt_tx.compute_txid(),
        dt_index,
        drt_data.address.to_vec(),
        takeback_hash,
        MAGIC_BYTES,
        BRIDGE_OUT_AMOUNT,
        agg_pubkey,
    )
    .map_err(|e| Error::TxBuilder(format!("Failed to build deposit transaction: {e}")))?;

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

fn sign_deposit_transaction(
    unsigned_tx: Transaction,
    prevout: &TxOut,
    takeback_hash: TapNodeHash,
    signers: &[EvenSecretKey],
) -> Result<Transaction, Error> {
    use bdk_wallet::bitcoin::sighash::{Prevouts, SighashCache};

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
