//! Deposit Transaction (DT) creation and signing functionality
//!
//! Handles the creation of deposit transactions that convert DRT (Deposit Request Transactions)
//! into actual bridge deposits with MuSig2 multi-signature support.

use bdk_wallet::bitcoin::{
    bip32::Xpriv,
    consensus::serialize,
    key::UntweakedPublicKey,
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
    Amount, Network, Psbt, TapNodeHash, TapSighashType, TxOut,
};
// Import utility functions that we'll need from mock-bridge patterns
use bdk_wallet::bitcoin::{
    script::PushBytesBuf, Address, ScriptBuf, Transaction, TxIn, TxOut as BitcoinTxOut,
    XOnlyPublicKey,
};
use pyo3::prelude::*;
use secp256k1::{All, Keypair, Secp256k1, SECP256K1};

use super::{
    musig_signer::MusigSigner,
    types::{AuxiliaryData, DepositTx, DepositTxMetadata, TaprootWitness},
};
use crate::{
    bridge::drt::DEPOSIT_REQUEST_DATA_STORAGE,
    constants::{BRIDGE_OUT_AMOUNT, GENERAL_WALLET_KEY_PATH, MAGIC_BYTES, NETWORK},
    error::Error,
    taproot::musig_aggregate_pks_inner,
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
    let signed_tx = create_deposit_transaction_inner(deposit_index, operator_keys)?;

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

    let (operator_secret_keys, agg_pubkey) = parse_operator_keys(&operator_keys)?;

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

    let metadata = AuxiliaryData::new(
        String::from_utf8(MAGIC_BYTES.to_vec()).expect("invalid magic bytes"),
        deposit_metadata,
    );

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
    use std::str::FromStr;

    use bdk_wallet::miniscript::{miniscript::Tap, Miniscript};

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

    psbt.clone()
        .extract_tx()
        .map_err(|e| Error::BridgeBuilder(format!("Transaction extraction failed: {}", e)))
}

/// Parses operator secret keys from hex strings
pub(crate) fn parse_operator_keys(
    operator_keys: &[String],
) -> Result<(Vec<Keypair>, XOnlyPublicKey), Error> {
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

            let mut sk = xp.private_key;
            let pk = secp256k1::PublicKey::from_secret_key(SECP256K1, &sk);

            // This is very important because datatool and bridge does this way.
            // (x,P) and (x,-P) don't add to same group element, so in order to be consistent
            // we are only choosing even one so
            // if not even
            if pk.serialize()[0] != 0x02 {
                // Flip to even-Y equivalent
                sk = sk.negate();
            }

            Keypair::from_secret_key(SECP256K1, &sk)
        })
        .collect();

    let x_only_keys: Vec<XOnlyPublicKey> = result
        .iter()
        .map(|pair| XOnlyPublicKey::from_keypair(pair).0)
        .collect();

    Ok((result, musig_aggregate_pks_inner(x_only_keys)?))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bdk_wallet::bitcoin::{
        address::AddressType,
        bip32::Xpriv,
        hashes::Hash,
        locktime::absolute::LockTime,
        opcodes::all::OP_RETURN,
        secp256k1::{schnorr, Message, SecretKey},
        Sequence,
    };

    use super::*;
    use crate::{
        bridge::types::DepositRequestData,
        constants::{BRIDGE_OUT_AMOUNT, GENERAL_WALLET_KEY_PATH, MAGIC_BYTES, NETWORK, XPRIV},
    };

    fn sample_deposit_request() -> DepositRequestData {
        use bdk_wallet::bitcoin::{Amount, OutPoint};

        DepositRequestData {
            deposit_request_outpoint: OutPoint::from_str(
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef:0",
            )
            .unwrap(),
            stake_index: 7,
            ee_address: vec![0x11; 20],
            total_amount: Amount::from_sat(1_000_001_000),
            x_only_public_key: XOnlyPublicKey::from_str(
                "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .unwrap(),
            // Not used for any logic in build_deposit_tx besides prevout script reference
            original_script_pubkey: ScriptBuf::new(),
        }
    }

    #[test]
    fn test_build_timelock_miniscript() {
        let xonly = XOnlyPublicKey::from_str(
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        )
        .unwrap();
        let script = build_timelock_miniscript(xonly).expect("miniscript builds");
        assert!(!script.as_bytes().is_empty());

        // Leaf hash should be 32 bytes
        let leaf_hash = TapNodeHash::from_script(&script, LeafVersion::TapScript);
        assert_eq!(leaf_hash.to_byte_array().len(), 32);
    }

    #[test]
    fn test_create_metadata_script_and_contents() {
        use bdk_wallet::bitcoin::Amount;

        let takeback_hash = TapNodeHash::from_byte_array([0xAA; 32]);
        let meta = DepositTxMetadata {
            stake_index: 0x01020304,
            ee_address: vec![0xDE; 20],
            takeback_hash,
            input_amount: Amount::from_sat(12345),
        };
        let aux = AuxiliaryData::new(String::from_utf8(MAGIC_BYTES.to_vec()).unwrap(), meta);
        let data = aux.to_vec();
        let script = create_metadata_script(&aux).expect("metadata script");

        let bytes = script.as_bytes();
        assert_eq!(bytes[0], OP_RETURN.to_u8());

        // Decode minimally: OP_RETURN, then opcode equals data length (<= 75 for our data)
        assert!(data.len() <= 75);
        assert_eq!(bytes[1] as usize, data.len());
        assert_eq!(&bytes[2..], &data);
    }

    #[test]
    fn test_build_taptree_empty_scripts() {
        // Build an internal key from XPRIV child to use as taproot internal key
        let xpriv = Xpriv::from_str(XPRIV).unwrap();
        let child = xpriv
            .derive_priv(SECP256K1, &GENERAL_WALLET_KEY_PATH)
            .unwrap();
        let sec = child.private_key;
        let pk = secp256k1::PublicKey::from_secret_key(SECP256K1, &sec);
        let xonly = XOnlyPublicKey::from(pk);

        let (addr, spend_info) = build_taptree(xonly, NETWORK, &[]).expect("taptree");
        assert_eq!(addr.address_type(), Some(AddressType::P2tr));
        assert!(spend_info.merkle_root().is_none());

        let spk = addr.script_pubkey();
        let sb = spk.as_bytes();
        // P2TR script: OP_1 (0x51) + 0x20 + 32-byte program
        assert_eq!(sb[0], 0x51);
        assert_eq!(sb[1], 0x20);
        assert_eq!(sb.len(), 34);
    }

    #[test]
    fn test_build_deposit_tx_structure() {
        let data = sample_deposit_request();

        // Prepare internal key from operator keys
        let (kps, agg) = parse_operator_keys(&[XPRIV.to_string()]).expect("keys parse");
        assert_eq!(kps.len(), 1);

        let deposit_tx = build_deposit_tx(&data, agg).expect("build deposit psbt");

        // Check PSBT structure: 1 input, 2 outputs (bridge + metadata)
        assert_eq!(deposit_tx.psbt().unsigned_tx.input.len(), 1);
        assert_eq!(deposit_tx.psbt().unsigned_tx.output.len(), 2);
        assert_eq!(deposit_tx.psbt().unsigned_tx.lock_time, LockTime::ZERO);

        // Output[0] is bridge-out amount
        assert_eq!(
            deposit_tx.psbt().unsigned_tx.output[0].value,
            BRIDGE_OUT_AMOUNT
        );

        // Output[1] should be OP_RETURN with metadata that contains MAGIC_BYTES
        let meta_spk = &deposit_tx.psbt().unsigned_tx.output[1].script_pubkey;
        let meta_bytes = meta_spk.as_bytes();
        assert_eq!(meta_bytes[0], OP_RETURN.to_u8());
        assert!(meta_bytes
            .windows(MAGIC_BYTES.len())
            .any(|w| w == MAGIC_BYTES));

        // Prevouts and witnesses
        assert_eq!(deposit_tx.prevouts().len(), 1);
        assert_eq!(deposit_tx.prevouts()[0].value, data.total_amount);

        // Witness should contain the takeback hash tweak computed from recovery key
        let takeback_script = build_timelock_miniscript(data.x_only_public_key).unwrap();
        let expected_tweak = TapNodeHash::from_script(&takeback_script, LeafVersion::TapScript);
        assert_eq!(
            deposit_tx.witnesses()[0],
            TaprootWitness::Tweaked {
                tweak: expected_tweak
            }
        );
    }

    #[test]
    fn test_finalize_and_extract_tx_adds_witness() {
        use bdk_wallet::bitcoin::{Amount, OutPoint, Witness};

        // Minimal PSBT with one input and two outputs
        let tx = Transaction {
            version: bdk_wallet::bitcoin::transaction::Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::from_str(
                    "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef:0",
                )
                .unwrap(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
        // Set required PSBT input fields
        psbt.inputs[0].witness_utxo = Some(TxOut {
            script_pubkey: ScriptBuf::new(),
            value: Amount::from_sat(2000),
        });
        psbt.inputs[0].sighash_type = Some(TapSighashType::Default.into());

        let prevouts = vec![TxOut {
            script_pubkey: ScriptBuf::new(),
            value: Amount::from_sat(2000),
        }];
        let witnesses = vec![TaprootWitness::Key];
        let mut dep = DepositTx::new(psbt, prevouts, witnesses);

        // Fabricate a valid Schnorr signature for witness (content isn't validated on extraction)
        let sec =
            SecretKey::from_str("1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap();
        let kp = secp256k1::Keypair::from_secret_key(SECP256K1, &sec);
        let msg = Message::from_digest([9u8; 32]);
        let sig64: schnorr::Signature = SECP256K1.sign_schnorr(&msg, &kp);
        let tap_sig = bdk_wallet::bitcoin::taproot::Signature {
            signature: sig64,
            sighash_type: TapSighashType::Default,
        };

        dep.psbt_mut().inputs[0].tap_key_sig = Some(tap_sig);

        let extracted = finalize_and_extract_tx(dep).expect("finalize and extract");
        assert_eq!(extracted.input.len(), 1);
        assert_eq!(extracted.input[0].witness.len(), 1);
        assert!(extracted.input[0].witness.iter().next().unwrap().is_empty());
    }

    #[test]
    fn test_parse_operator_keys_from_xpriv() {
        // Should parse and derive an even-y keypair and aggregated x-only pk
        let (pairs, agg1) = parse_operator_keys(&[XPRIV.to_string()]).expect("parse");
        assert_eq!(pairs.len(), 1);

        // Derived public key should be even (compressed prefix 0x02)
        let full_pk = secp256k1::PublicKey::from_secret_key(SECP256K1, &pairs[0].secret_key());
        assert_eq!(full_pk.serialize()[0], 0x02);

        // Aggregated key should be deterministic for the same input
        let (_, agg2) = parse_operator_keys(&[XPRIV.to_string()]).expect("parse");
        assert_eq!(agg1, agg2);
    }

    #[test]
    fn test_create_deposit_transaction_inner_index_out_of_bounds() {
        // No entries in storage; any index should be out of bounds
        let result = create_deposit_transaction_inner(0, vec![XPRIV.to_string()]);
        assert!(matches!(result, Err(Error::BridgeBuilder(msg)) if msg.contains("out of bounds")));
    }
}
