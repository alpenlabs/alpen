//! Deposit Transaction (DT) creation and signing functionality
//!
//! Handles the creation of deposit transactions that convert DRT (Deposit Request Transactions)
//! into actual bridge deposits with MuSig2 multi-signature support.

use std::str::FromStr;

use bdk_wallet::{
    bitcoin::{
        consensus::{deserialize, serialize},
        key::UntweakedPublicKey,
        locktime::absolute::LockTime,
        opcodes::all::OP_RETURN,
        script::{Builder, PushBytesBuf},
        taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
        transaction::Version,
        Address, Amount, Network, OutPoint, Psbt, ScriptBuf, Sequence, TapNodeHash, TapSighashType,
        Transaction, TxIn, TxOut, TxOut as BitcoinTxOut, Witness, XOnlyPublicKey,
    },
    miniscript::{miniscript::Tap, Miniscript},
};
use make_buf::make_buf;
use pyo3::{exceptions::PyValueError, prelude::*};
use secp256k1::{All, Secp256k1, SECP256K1};
use strata_crypto::EvenSecretKey;
use strata_l1tx::utils::generate_taproot_address;
use strata_primitives::{buf::Buf32, constants::RECOVER_DELAY, l1::DepositRequestInfo};

use super::{musig_signer::MusigSigner, types::DepositTxMetadata};
use crate::{
    constants::{BRIDGE_OUT_AMOUNT, MAGIC_BYTES, NETWORK},
    error::Error,
    parse::{parse_drt, parse_operator_keys},
};

/// Creates a deposit transaction (DT) from raw DRT transaction bytes
///
/// # Arguments
/// * `tx_bytes` - Raw DRT transaction bytes
/// * `operator_keys` - Vector of operator secret keys as bytes (78 bytes each)
/// * `dt_index` - Deposit transaction index for metadata
///
/// # Returns
/// * `PyResult<Vec<u8>>` - The signed and serialized deposit transaction

#[pyfunction]
pub(crate) fn create_deposit_transaction(
    tx_bytes: Vec<u8>,
    operator_keys: Vec<[u8; 78]>,
    dt_index: u32,
) -> PyResult<Vec<u8>> {
    // Parse transaction
    let parsed_tx = deserialize::<Transaction>(&tx_bytes)
        .map_err(|e| PyValueError::new_err(format!("invalid transaction: {e}")))?;

    // Original logic using fixed_keys
    let signers = parse_operator_keys(operator_keys.as_ref())?;

    let pubkeys = signers
        .iter()
        .map(|kp| Buf32::from(kp.x_only_public_key(SECP256K1).0))
        .collect::<Vec<_>>();

    let (address, agg_pubkey) =
        generate_taproot_address(&pubkeys, NETWORK).map_err(|e| Error::TxBuilder(e.to_string()))?;

    let drt_data = parse_drt(&parsed_tx, address, agg_pubkey)?;

    let signed_tx =
        create_deposit_transaction_inner(&parsed_tx, drt_data, dt_index, signers, agg_pubkey)?;

    Ok(serialize(&signed_tx))
}

/// Internal implementation of deposit transaction creation
fn create_deposit_transaction_inner(
    drt_tx: &Transaction,
    drt_data: DepositRequestInfo,
    dt_index: u32,
    signers: Vec<EvenSecretKey>,
    agg_pubkey: XOnlyPublicKey,
) -> Result<Transaction, Error> {
    // Create the deposit transaction PSBT
    let (mut psbt, prevouts, tweak) = build_deposit_tx(drt_tx, &drt_data, dt_index, agg_pubkey)?;

    // Sign with MuSig2
    let signer = MusigSigner;
    let signature = signer.sign_deposit_psbt(&psbt, &prevouts, tweak, signers, 0)?;

    // Add signature to PSBT
    if let Some(input) = psbt.inputs.get_mut(0) {
        input.tap_key_sig = Some(signature);
    } else {
        return Err(Error::TxBuilder("Input index out of bounds".to_string()));
    }

    // Finalize and extract transaction
    finalize_and_extract_tx(psbt)
}

fn build_taptree(
    internal_key: UntweakedPublicKey,
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
    drt_tx: &Transaction,
    drt_data: &DepositRequestInfo,
    dt_index: u32,
    internal_key: UntweakedPublicKey,
) -> Result<(Psbt, Vec<TxOut>, Option<TapNodeHash>), Error> {
    let deposit_request_output = drt_tx.output.first().expect("valid DRT Transaction");

    let prevouts = vec![TxOut {
        script_pubkey: deposit_request_output.script_pubkey.clone(),
        value: Amount::from_sat(drt_data.amt),
    }];

    // Create the inputs
    let tx_ins = vec![TxIn {
        previous_output: OutPoint::new(drt_tx.compute_txid(), 0),
        script_sig: ScriptBuf::default(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    }];

    let key =
        XOnlyPublicKey::from_slice(&drt_data.take_back_leaf_hash).expect("Valid XOnlyPublicKey");

    let takeback_script = build_timelock_miniscript(key)?;
    let takeback_script_hash = TapNodeHash::from_script(&takeback_script, LeafVersion::TapScript);

    let deposit_metadata = DepositTxMetadata {
        stake_index: dt_index,
        ee_address: drt_data.address.to_vec(),
        takeback_hash: takeback_script_hash,
        input_amount: Amount::from_sat(drt_data.amt),
    };

    let metadata_script = create_metadata_script_direct(&deposit_metadata)?;
    let metadata_amount = Amount::from_int_btc(0);

    let (bridge_address, _) = build_taptree(internal_key, NETWORK, &[])?;
    let bridge_in_script_pubkey = bridge_address.script_pubkey();

    let tx_outs = vec![
        BitcoinTxOut {
            script_pubkey: bridge_in_script_pubkey,
            value: BRIDGE_OUT_AMOUNT,
        },
        BitcoinTxOut {
            script_pubkey: metadata_script,
            value: metadata_amount,
        },
    ];

    let unsigned_tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: tx_ins,
        output: tx_outs,
    };

    let mut psbt = Psbt::from_unsigned_tx(unsigned_tx)
        .map_err(|e| Error::TxBuilder(format!("Failed to create PSBT: {}", e)))?;

    for (i, input) in psbt.inputs.iter_mut().enumerate() {
        input.witness_utxo = Some(prevouts[i].clone());
        input.sighash_type = Some(TapSighashType::Default.into());
    }

    Ok((psbt, prevouts, Some(takeback_script_hash)))
}

/// Builds the timelock miniscript for takeback functionality
fn build_timelock_miniscript(recovery_xonly_pubkey: XOnlyPublicKey) -> Result<ScriptBuf, Error> {
    let script = format!(
        "and_v(v:pk({}),older({}))",
        recovery_xonly_pubkey, RECOVER_DELAY
    );
    let miniscript = Miniscript::<XOnlyPublicKey, Tap>::from_str(&script)
        .map_err(|e| Error::TxBuilder(format!("Failed to create miniscript: {}", e)))?;
    Ok(miniscript.encode())
}

/// Creates the metadata script for OP_RETURN directly from metadata
fn create_metadata_script_direct(metadata: &DepositTxMetadata) -> Result<ScriptBuf, Error> {
    let buf = make_buf! {
        (MAGIC_BYTES, 4),
        (&metadata.stake_index.to_be_bytes(), 4),
        (&metadata.ee_address, 20),
        (&metadata.takeback_hash.as_ref(), 32),
        (&metadata.input_amount.to_sat().to_be_bytes(), 8)
    };

    let push_data = PushBytesBuf::from(buf);

    Ok(Builder::new()
        .push_opcode(OP_RETURN)
        .push_slice(push_data)
        .into_script())
}

/// Finalizes the PSBT and extracts the signed transaction
fn finalize_and_extract_tx(mut psbt: Psbt) -> Result<Transaction, Error> {
    // Finalize all inputs
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bdk_wallet::bitcoin::{
        bip32::Xpriv,
        locktime::absolute::LockTime,
        opcodes::all::OP_RETURN,
        secp256k1::{schnorr, Message, SecretKey},
        taproot::Signature,
        Amount, OutPoint, Sequence, Witness,
    };
    use secp256k1::SECP256K1;
    use strata_primitives::constants::STRATA_OP_WALLET_DERIVATION_PATH;

    use super::*;
    use crate::constants::{BRIDGE_OUT_AMOUNT, MAGIC_BYTES, XPRIV};

    #[test]
    fn test_build_deposit_tx_structure() {
        // Create test data directly without DepositRequestData
        let outpoint = OutPoint::from_str(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef:0",
        )
        .unwrap();
        let ee_address = vec![0x11; 20];
        let total_amount = Amount::from_sat(1_000_001_000);

        // Create a mock DRT transaction
        let drt_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: outpoint,
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: total_amount,
                script_pubkey: ScriptBuf::new(),
            }],
        };

        // Create deposit request info from the data
        let drt_data = DepositRequestInfo {
            amt: total_amount.to_sat(),
            address: ee_address.clone(),
            take_back_leaf_hash: [
                0x79, 0xBE, 0x66, 0x7E, 0xF9, 0xDC, 0xBB, 0xAC, 0x55, 0xA0, 0x62, 0x95, 0xCE, 0x87,
                0x0B, 0x07, 0x02, 0x9B, 0xFC, 0xDB, 0x2D, 0xCE, 0x28, 0xD9, 0x59, 0xF2, 0x81, 0x5B,
                0x16, 0xF8, 0x17, 0x98,
            ],
        };

        // Create internal key
        let xpriv = Xpriv::from_str(XPRIV).unwrap();
        let child = xpriv
            .derive_priv(SECP256K1, &STRATA_OP_WALLET_DERIVATION_PATH)
            .unwrap();
        let internal_key = UntweakedPublicKey::from(child.private_key.public_key(SECP256K1));

        let (psbt, prevouts, tweak) =
            build_deposit_tx(&drt_tx, &drt_data, 0, internal_key).expect("build deposit psbt");

        // Check PSBT structure: 1 input, 2 outputs (bridge + metadata)
        assert_eq!(psbt.unsigned_tx.input.len(), 1);
        assert_eq!(psbt.unsigned_tx.output.len(), 2);
        assert_eq!(psbt.unsigned_tx.lock_time, LockTime::ZERO);

        // Output[0] is bridge-out amount
        assert_eq!(psbt.unsigned_tx.output[0].value, BRIDGE_OUT_AMOUNT);

        // Output[1] should be OP_RETURN with metadata that contains MAGIC_BYTES
        let meta_spk = &psbt.unsigned_tx.output[1].script_pubkey;
        let meta_bytes = meta_spk.as_bytes();
        assert_eq!(meta_bytes[0], OP_RETURN.to_u8());
        assert!(meta_bytes
            .windows(MAGIC_BYTES.len())
            .any(|w| w == MAGIC_BYTES));

        // Prevouts
        assert_eq!(prevouts.len(), 1);
        assert_eq!(prevouts[0].value, total_amount);

        // Tweak should be Some (computed from the mock takeback hash)
        assert!(tweak.is_some());
    }

    #[test]
    fn test_finalize_and_extract_tx_adds_witness() {
        // Minimal PSBT with one input and two outputs
        let tx = Transaction {
            version: Version::TWO,
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

        // No need for witnesses wrapper, work directly with PSBT

        // Fabricate a valid Schnorr signature for witness (content isn't validated on extraction)
        let sec = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let kp = secp256k1::Keypair::from_secret_key(SECP256K1, &sec);
        let msg = Message::from_digest([9u8; 32]);
        let sig64: schnorr::Signature = SECP256K1.sign_schnorr(&msg, &kp);
        let tap_sig = Signature::from_slice(&sig64.serialize()).unwrap();

        psbt.inputs[0].tap_key_sig = Some(tap_sig);

        let extracted = finalize_and_extract_tx(psbt).expect("finalize and extract");
        assert_eq!(extracted.input.len(), 1);
        assert_eq!(extracted.input[0].witness.len(), 1);
        // The witness should contain the signature (64 bytes for Schnorr)
        assert_eq!(extracted.input[0].witness.iter().next().unwrap().len(), 64);
    }
}
