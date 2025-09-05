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
        taproot::{TaprootBuilder, TaprootSpendInfo},
        transaction::Version,
        Address, Amount, Network, OutPoint, Psbt, ScriptBuf, TapNodeHash, TapSighashType,
        Transaction, TxIn, TxOut, TxOut as BitcoinTxOut, XOnlyPublicKey,
    },
    miniscript::{miniscript::Tap, Miniscript},
};
use make_buf::make_buf;
use pyo3::prelude::*;
use secp256k1::{hashes::Hash, All, Secp256k1, SECP256K1};
use strata_crypto::EvenSecretKey;
use strata_l1tx::utils::generate_taproot_address;
use strata_primitives::{buf::Buf32, constants::RECOVER_DELAY, l1::DepositRequestInfo};

use super::{musig_signer::MusigSigner, types::DepositTxMetadata};
use crate::{
    constants::{BRIDGE_OUT_AMOUNT, MAGIC_BYTES, NETWORK},
    error::Error,
    parse::{parse_drt, parse_operator_keys},
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
    tx_bytes: Vec<u8>,
    operator_keys: Vec<[u8; 78]>,
    dt_index: u32,
) -> PyResult<Vec<u8>> {
    let parsed_tx = deserialize::<Transaction>(&tx_bytes).expect("valid serialized transaction");

    let signers = parse_operator_keys(&operator_keys)?;

    let pubkeys = signers
        .iter()
        .map(|kp| Buf32::from(kp.x_only_public_key(SECP256K1).0))
        .collect::<Vec<_>>();

    let (address, agg_pubkey) =
        generate_taproot_address(&pubkeys, NETWORK).map_err(|e| Error::TxBuilder(e.to_string()))?;

    let drt_data = parse_drt(&parsed_tx, address, agg_pubkey)?;

    let signed_tx =
        create_deposit_transaction_inner(&parsed_tx, drt_data, dt_index, signers, agg_pubkey)?;

    let signed_tx = serialize(&signed_tx);
    Ok(signed_tx)
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
        sequence: bdk_wallet::bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: bdk_wallet::bitcoin::Witness::new(),
    }];

    let takeback_script_hash =
        TapNodeHash::from_slice(&drt_data.take_back_leaf_hash).expect("valid Tap node hash");

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
        .map_err(|e| Error::TxBuilder(format!("Transaction extraction failed: {}", e)))
}
