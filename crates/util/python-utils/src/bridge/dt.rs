//! Deposit Transaction (DT) creation and signing functionality
//!
//! Handles the creation of deposit transactions that convert DRT (Deposit Request Transactions)
//! into actual bridge deposits with MuSig2 multi-signature support.

use bdk_wallet::bitcoin::{
    bip32::Xpriv,  consensus::serialize, key::UntweakedPublicKey, taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo}, Amount, Network, Psbt, TapNodeHash, TapSighashType, TxOut
};
use pyo3::prelude::*;
use secp256k1::{All, Keypair, Secp256k1, SECP256K1};

use crate::{
    bridge::drt::DEPOSIT_REQUEST_DATA_STORAGE, constants::{ BRIDGE_OUT_AMOUNT, GENERAL_WALLET_KEY_PATH, MAGIC_BYTES, NETWORK}, error::Error, taproot::musig_aggregate_pks_inner
};

use super::{
    musig_signer::MusigSigner,
    types::{
        AuxiliaryData, DepositTx, DepositTxMetadata, TaprootWitness,
    },
};

// Import utility functions that we'll need from mock-bridge patterns
use bdk_wallet::bitcoin::{
    script::PushBytesBuf, Address, ScriptBuf, Transaction, TxIn, TxOut as BitcoinTxOut,
    XOnlyPublicKey,
};

/// Creates a deposit transaction (DT) from deposit request data
///
/// # Arguments
/// * `deposit_index` - Index of the deposit request data in the storage
/// * `operator_keys` - Vector of operator secret keys as hex strings for MuSig2 signing
///
/// # Returns
/// * `PyResult<Vec<u8>>` - The signed and serialized deposit transaction
#[pyfunction]
pub(crate) fn create_deposit_transaction(
    deposit_index: u32,
    operator_keys: Vec<String>,
) -> PyResult<Vec<u8>> {


    let signed_tx = create_deposit_transaction_inner(
        deposit_index,
        operator_keys,
    )?;

    let signed_tx = serialize(&signed_tx);
    Ok(signed_tx)
}

/// Internal implementation of deposit transaction creation
fn create_deposit_transaction_inner(
    deposit_index: u32,
    operator_keys: Vec<String>,
) -> Result<Transaction, Error> {
    // Fetch deposit request data from LazyLock storage
    let deposit_request_data = {
        let storage = DEPOSIT_REQUEST_DATA_STORAGE.lock().unwrap();
        let index = deposit_index as usize;

        if index >= storage.len() {
            return Err(Error::BridgeBuilder(format!(
                "Deposit index {} out of bounds. Storage contains {} items",
                deposit_index,
                storage.len()
            )));
        }

        storage[index].clone()
    };

    let agg_pubkey = parse_keys(&operator_keys)?;
    let operator_secret_keys = parse_operator_keys(operator_keys)?;

    // Create the deposit transaction PSBT
    let mut deposit_tx = build_deposit_tx(&deposit_request_data, agg_pubkey)?;

    // Sign with MuSig2
    let signer = MusigSigner::new();
    let signature = signer.sign_deposit_psbt(&deposit_tx, operator_secret_keys, 0)?;

    // Add signature to PSBT
    if let Some(input) = deposit_tx.psbt_mut().inputs.get_mut(0) {
        input.tap_key_sig = Some(signature);
    } else {
        return Err(Error::BridgeBuilder(
            "Input index out of bounds".to_string(),
        ));
    }

    // Finalize and extract transaction
    finalize_and_extract_tx(deposit_tx)
}

fn build_taptree(
    internal_key: bdk_wallet::bitcoin::key::UntweakedPublicKey,
    network: Network,
    scripts: &[ScriptBuf],
) -> Result<(Address, TaprootSpendInfo), Error> {
    let mut taproot_builder = TaprootBuilder::new();

    let num_scripts = scripts.len();

    let max_depth = if num_scripts > 1 {
        (num_scripts - 1).ilog2() + 1
    } else {
        0
    };

    let max_num_scripts = 2usize.pow(max_depth);

    let num_penultimate_scripts = max_num_scripts.saturating_sub(num_scripts);
    let num_deepest_scripts = num_scripts.saturating_sub(num_penultimate_scripts);

    for (script_idx, script) in scripts.iter().enumerate() {
        let depth = if script_idx < num_deepest_scripts {
            max_depth as u8
        } else {
            (max_depth - 1) as u8
        };

        taproot_builder = taproot_builder.add_leaf(depth, script.clone()).unwrap();
    }

    let secp = Secp256k1::<All>::new();
    let spend_info = taproot_builder.finalize(&secp, internal_key).unwrap();
    let merkle_root = spend_info.merkle_root();

    Ok((
        Address::p2tr(&secp, internal_key, merkle_root, network),
        spend_info,
    ))
}

/// Builds the deposit transaction PSBT from deposit request data
fn build_deposit_tx(
    data: &crate::bridge::types::DepositRequestData,
    internal_key: UntweakedPublicKey,
) -> Result<DepositTx, Error> {
    let deposit_amount = BRIDGE_OUT_AMOUNT;

    let prevouts = vec![TxOut {
        script_pubkey: data.original_script_pubkey.clone(),
        value: data.total_amount,
    }];

    // Create the inputs
    let outpoint = data.deposit_request_outpoint;
    let tx_ins = vec![TxIn {
        previous_output: outpoint,
        script_sig: ScriptBuf::default(),
        sequence: bdk_wallet::bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: bdk_wallet::bitcoin::Witness::new(),
    }];

    // Create and validate the OP_RETURN metadata
    let takeback_script = build_timelock_miniscript(data.x_only_public_key)?;
    let takeback_script_hash = TapNodeHash::from_script(&takeback_script, LeafVersion::TapScript);

    let deposit_metadata = DepositTxMetadata {
        stake_index: data.stake_index,
        ee_address: data.ee_address.to_vec(),
        takeback_hash: takeback_script_hash,
        input_amount: data.total_amount,
    };

    let metadata = AuxiliaryData::new(String::from_utf8(MAGIC_BYTES.to_vec()).expect("invalid magic bytes"), deposit_metadata);

    let metadata_script = create_metadata_script(&metadata)?;
    let metadata_amount = Amount::from_int_btc(0);

    let (bridge_address, _) = build_taptree(internal_key, NETWORK, &[])?;
    let bridge_in_script_pubkey = bridge_address.script_pubkey();

    let tx_outs = vec![
        BitcoinTxOut {
            script_pubkey: bridge_in_script_pubkey,
            value: deposit_amount,
        },
        BitcoinTxOut {
            script_pubkey: metadata_script,
            value: metadata_amount,
        },
    ];

    let unsigned_tx = Transaction {
        version: bdk_wallet::bitcoin::transaction::Version::TWO,
        lock_time: bdk_wallet::bitcoin::locktime::absolute::LockTime::ZERO,
        input: tx_ins,
        output: tx_outs,
    };

    let mut psbt = Psbt::from_unsigned_tx(unsigned_tx)
        .map_err(|e| Error::BridgeBuilder(format!("Failed to create PSBT: {}", e)))?;

    for (i, input) in psbt.inputs.iter_mut().enumerate() {
        input.witness_utxo = Some(prevouts[i].clone());
        input.sighash_type = Some(TapSighashType::Default.into());
    }

    let witnesses = vec![TaprootWitness::Tweaked {
        tweak: takeback_script_hash,
    }];

    Ok(DepositTx::new(psbt, prevouts, witnesses))
}

/// Builds the timelock miniscript for takeback functionality
fn build_timelock_miniscript(recovery_xonly_pubkey: XOnlyPublicKey) -> Result<ScriptBuf, Error> {
    use bdk_wallet::miniscript::{miniscript::Tap, Miniscript};
    use std::str::FromStr;

    let script = format!("and_v(v:pk({}),older({}))", recovery_xonly_pubkey, 1008);
    let miniscript = Miniscript::<XOnlyPublicKey, Tap>::from_str(&script)
        .map_err(|e| Error::BridgeBuilder(format!("Failed to create miniscript: {}", e)))?;
    Ok(miniscript.encode())
}

/// Creates the metadata script for OP_RETURN
fn create_metadata_script(metadata: &AuxiliaryData) -> Result<ScriptBuf, Error> {
    use bdk_wallet::bitcoin::{opcodes::all::OP_RETURN, script::Builder};

    let data = metadata.to_vec();
    let push_data = PushBytesBuf::try_from(data)
        .map_err(|_| Error::BridgeBuilder("Metadata too large for OP_RETURN".to_string()))?;

    Ok(Builder::new()
        .push_opcode(OP_RETURN)
        .push_slice(push_data)
        .into_script())
}

/// Finalizes the PSBT and extracts the signed transaction
fn finalize_and_extract_tx(mut deposit_tx: DepositTx) -> Result<Transaction, Error> {
    let psbt = deposit_tx.psbt_mut();

    // Finalize all inputs
    for input in &mut psbt.inputs {
        if input.tap_key_sig.is_some() {
            input.final_script_witness = Some(bdk_wallet::bitcoin::Witness::new());
            if let Some(sig) = &input.tap_key_sig {
                input
                    .final_script_witness
                    .as_mut()
                    .unwrap()
                    .push(sig.to_vec());
            }
        }
    }

    psbt.clone().extract_tx()
        .map_err(|e| Error::BridgeBuilder(format!("Transaction extraction failed: {}", e)))
}

/// Parses operator secret keys from hex strings
pub(crate) fn parse_keys(operator_keys: &[String]) -> Result<XOnlyPublicKey, Error> {
    use std::str::FromStr;

    let result: Vec<Keypair> = operator_keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let xpriv = Xpriv::from_str(key)
                .map_err(|e| Error::BridgeBuilder(format!("Invalid operator key {}: {}", i, e)))
                .unwrap();

            let xp = xpriv
                .derive_priv(SECP256K1, &GENERAL_WALLET_KEY_PATH)
                .expect("good child key");

            Keypair::from_secret_key(SECP256K1, &xp.private_key)
        })
        .collect();


    let x_only_keys: Vec<XOnlyPublicKey> = result.iter().map(|pair| XOnlyPublicKey::from_keypair(pair).0).collect();


    musig_aggregate_pks_inner(x_only_keys)
}

/// Parses operator secret keys from hex strings
pub(crate) fn parse_operator_keys(operator_keys: Vec<String>) -> Result<Vec<Keypair>, Error> {
    use std::str::FromStr;

    Ok(operator_keys
        .into_iter()
        .enumerate()
        .map(|(i, key)| {

            let xpriv = Xpriv::from_str(&key)
                .map_err(|e| Error::BridgeBuilder(format!("Invalid operator key {}: {}", i, e)))
                .unwrap();

            let xp = xpriv
                .derive_priv(SECP256K1, &GENERAL_WALLET_KEY_PATH)
                .expect("good child key");

            let mut sk = xp.private_key;
            let pk = secp256k1::PublicKey::from_secret_key(SECP256K1, &sk);

            // if not even
            if pk.serialize()[0] != 0x02 {
                // Flip to even-Y equivalent
                sk = sk.negate();
            }

            Keypair::from_secret_key(SECP256K1, &sk)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_operator_keys() {
        let keys = vec![
            "1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            "2222222222222222222222222222222222222222222222222222222222222222".to_string(),
        ];
        assert!(parse_operator_keys(keys).is_ok());
    }
}
