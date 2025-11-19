//! Deposit transaction builders and test utilities

use bitcoin::{
    Address, Amount, Network, OutPoint, ScriptBuf, Sequence, TapNodeHash, TapSighashType,
    Transaction, TxIn, TxOut, Witness,
    absolute::LockTime,
    hashes::Hash,
    key::UntweakedPublicKey,
    script::{Builder, PushBytesBuf},
    taproot::{TaprootBuilder, TaprootSpendInfo},
    transaction::Version,
    XOnlyPublicKey,
};
use strata_codec::encode_to_vec;
use strata_crypto::{
    EvenSecretKey,
    test_utils::schnorr::{create_agg_pubkey_from_privkeys, create_musig2_signature},
};
use strata_l1_txfmt::{ParseConfig, TagDataRef};
use strata_primitives::constants::RECOVER_DELAY;

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE},
    deposit::DepositInfo,
    test_utils::create_tagged_payload as create_tagged_payload_util,
};

/// Error type for deposit transaction building
#[derive(Debug, Clone, thiserror::Error)]
pub enum DepositTxBuilderError {
    #[error("Transaction builder error: {0}")]
    TxBuilder(String),

    #[error("Input index out of bounds")]
    InputIndexOutOfBounds,

    #[error("Invalid deposit request transaction")]
    InvalidDRT,

    #[error("SPS-50 format error: {0}")]
    TxFmt(String),
}

/// Builds unsigned deposit transaction (no DRT parsing or signing)
///
/// This is the primary API for building deposit transactions. It takes parsed
/// deposit request data and creates an unsigned transaction that can be signed
/// separately.
///
/// # Arguments
/// * `drt_txid` - Transaction ID of the deposit request transaction
/// * `dt_index` - Deposit transaction index for metadata
/// * `ee_address` - Execution environment address where sBTC will be minted
/// * `takeback_hash` - Taproot hash for the takeback script
/// * `magic_bytes` - Protocol magic bytes for SPS-50 tagging
/// * `bridge_out_amount` - Amount to lock in the bridge
/// * `agg_pubkey` - Aggregated public key of operators
///
/// # Returns
/// An unsigned `Transaction` ready for signing
pub fn build_deposit_transaction(
    drt_txid: bitcoin::Txid,
    dt_index: u32,
    ee_address: Vec<u8>,
    takeback_hash: TapNodeHash,
    magic_bytes: &[u8; 4],
    bridge_out_amount: Amount,
    agg_pubkey: XOnlyPublicKey,
) -> Result<Transaction, DepositTxBuilderError> {
    let deposit_metadata = DepositTxMetadata {
        stake_index: dt_index,
        ee_address,
        takeback_hash,
    };

    // Create the inputs
    let tx_ins = vec![TxIn {
        previous_output: OutPoint::new(drt_txid, 0),
        script_sig: ScriptBuf::default(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    }];

    let metadata_script = create_metadata_script(&deposit_metadata, magic_bytes)?;
    let metadata_amount = Amount::from_int_btc(0);

    let (bridge_address, _) = build_taptree(agg_pubkey, Network::Regtest, &[])?;
    let bridge_in_script_pubkey = bridge_address.script_pubkey();

    let tx_outs = vec![
        TxOut {
            script_pubkey: metadata_script,
            value: metadata_amount,
        },
        TxOut {
            script_pubkey: bridge_in_script_pubkey,
            value: bridge_out_amount,
        },
    ];

    Ok(Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: tx_ins,
        output: tx_outs,
    })
}

/// Builds the timelock script for takeback functionality
///
/// Creates a script that allows the depositor to reclaim funds after a timeout.
/// The script format is: `<pubkey> OP_CHECKSIGVERIFY <delay> OP_CSV`
///
/// # Arguments
/// * `recovery_pubkey_bytes` - The depositor's x-only public key (32 bytes)
///
/// # Returns
/// A `ScriptBuf` containing the timelock script
pub fn build_timelock_script(recovery_pubkey_bytes: &[u8; 32]) -> Result<ScriptBuf, DepositTxBuilderError> {
    // Manual construction of the timelock script: OP_PUSH(pubkey) OP_CHECKSIGVERIFY OP_PUSH(delay) OP_CHECKSEQUENCEVERIFY
    // This is equivalent to: and_v(v:pk(pubkey),older(delay))
    use bitcoin::opcodes::all::{OP_CHECKSIGVERIFY, OP_CSV};

    let script = Builder::new()
        .push_slice(recovery_pubkey_bytes)
        .push_opcode(OP_CHECKSIGVERIFY)
        .push_int(RECOVER_DELAY as i64)
        .push_opcode(OP_CSV)
        .into_script();

    Ok(script)
}

/// Creates a test deposit transaction with MuSig2 signatures
///
/// This is a convenience function for testing that creates a fully signed
/// deposit transaction. It handles all the complexity of MuSig2 aggregation
/// and signing.
///
/// # Arguments
/// * `deposit_info` - Parsed deposit information
/// * `operators_privkeys` - Private keys of operators for signing
///
/// # Returns
/// A fully signed `Transaction` ready for broadcast
pub fn create_test_deposit_tx(
    deposit_info: &DepositInfo,
    operators_privkeys: &[EvenSecretKey],
) -> Transaction {
    // Create auxiliary data in the expected format for deposit transactions
    let aux_data = encode_to_vec(deposit_info.header_aux()).unwrap();
    let td = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, aux_data).unwrap();
    let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&td.as_ref())
        .unwrap();

    let secp = Secp256k1::new();
    let aggregated_xonly = create_agg_pubkey_from_privkeys(operators_privkeys);

    // Use aggregated key for deposit output
    let deposit_script = ScriptBuf::new_p2tr(&secp, aggregated_xonly, None);

    // Create the UTXO being spent (DRT output)
    let merkle_root =
        TapNodeHash::from_byte_array(deposit_info.header_aux().drt_tapscript_merkle_root());
    let drt_script_pubkey = ScriptBuf::new_p2tr(&secp, aggregated_xonly, Some(merkle_root));

    let deposit_amount: Amount = deposit_info.amt().into();
    let prev_txout = TxOut {
        value: deposit_amount,
        script_pubkey: drt_script_pubkey,
    };

    // Create unsigned transaction
    let unsigned_tx = Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![
            // OP_RETURN output at index 0
            TxOut {
                value: Amount::ZERO,
                script_pubkey: sps_50_script,
            },
            // Deposit output at index 1
            TxOut {
                value: deposit_amount,
                script_pubkey: deposit_script,
            },
        ],
    };

    // Create MuSig2 signature
    let prevouts = [prev_txout];
    let signature = create_musig2_signature_for_tx(
        &unsigned_tx,
        &prevouts,
        operators_privkeys,
        Some(merkle_root.to_byte_array()),
    );

    // Return signed transaction
    Transaction {
        version: unsigned_tx.version,
        lock_time: unsigned_tx.lock_time,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[signature.serialize().as_slice()]),
        }],
        output: unsigned_tx.output,
    }
}

/// Metadata for deposit transaction OP_RETURN
#[derive(Debug, Clone)]
pub(super) struct DepositTxMetadata {
    pub stake_index: u32,
    pub ee_address: Vec<u8>,
    pub takeback_hash: TapNodeHash,
}

fn build_taptree(
    internal_key: UntweakedPublicKey,
    network: Network,
    scripts: &[ScriptBuf],
) -> Result<(Address, TaprootSpendInfo), DepositTxBuilderError> {
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

        taproot_builder = taproot_builder
            .add_leaf(depth, script.clone())
            .map_err(|e| DepositTxBuilderError::TxBuilder(format!("taproot builder: {e}")))?;
    }

    let secp = Secp256k1::<All>::new();
    let spend_info = taproot_builder
        .finalize(&secp, internal_key)
        .map_err(|_| DepositTxBuilderError::TxBuilder("taproot finalization failed".to_string()))?;
    let merkle_root = spend_info.merkle_root();

    Ok((
        Address::p2tr(&secp, internal_key, merkle_root, network),
        spend_info,
    ))
}

fn create_metadata_script(
    metadata: &DepositTxMetadata,
    magic_bytes: &[u8; 4],
) -> Result<ScriptBuf, DepositTxBuilderError> {
    // Build auxiliary data (everything after subprotocol_id and tx_type)
    let mut aux_data = Vec::new();
    aux_data.extend_from_slice(&metadata.stake_index.to_be_bytes()); // 4 bytes
    aux_data.extend_from_slice(metadata.takeback_hash.as_ref());     // 32 bytes
    aux_data.extend_from_slice(&metadata.ee_address);                // 20 bytes

    // Create SPS-50 tagged data
    let tag_data = TagDataRef::new(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, &aux_data)
        .map_err(|e| DepositTxBuilderError::TxFmt(e.to_string()))?;

    // Encode to OP_RETURN script using ParseConfig
    let op_return_script = ParseConfig::new(*magic_bytes)
        .encode_script_buf(&tag_data)
        .map_err(|e| DepositTxBuilderError::TxFmt(e.to_string()))?;

    Ok(op_return_script)
}

/// Helper to create MuSig2 signature for test transactions
fn create_musig2_signature_for_tx(
    tx: &Transaction,
    prevouts: &[TxOut],
    operators_privkeys: &[EvenSecretKey],
    tweak: Option<[u8; 32]>,
) -> bitcoin::secp256k1::schnorr::Signature {
    use bitcoin::sighash::{Prevouts, SighashCache};

    let prevouts_ref = Prevouts::All(prevouts);
    let mut sighash_cache = SighashCache::new(tx);
    let sighash = sighash_cache
        .taproot_key_spend_signature_hash(0, &prevouts_ref, TapSighashType::Default)
        .unwrap();

    let msg = sighash.to_byte_array();
    create_musig2_signature(operators_privkeys, &msg, tweak).into()
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{SECP256K1, SecretKey};

    use super::*;

    fn create_test_drt() -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1_000_000_000),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    fn create_test_operator_keys() -> Vec<EvenSecretKey> {
        vec![
            EvenSecretKey::from(SecretKey::from_slice(&[1u8; 32]).unwrap()),
            EvenSecretKey::from(SecretKey::from_slice(&[2u8; 32]).unwrap()),
        ]
    }

    #[test]
    fn test_deposit_metadata_script_creation() {
        let metadata = DepositTxMetadata {
            stake_index: 42,
            ee_address: vec![0x11; 20],
            takeback_hash: TapNodeHash::from_byte_array([0x22; 32]),
        };

        let magic_bytes = b"TEST";
        let script = create_metadata_script(&metadata, magic_bytes).unwrap();

        // Verify it's an OP_RETURN script
        assert!(script.is_op_return());
        let bytes = script.as_bytes();
        assert_eq!(bytes[0], OP_RETURN.to_u8());
    }

    #[test]
    fn test_build_deposit_transaction() {
        let drt_tx = create_test_drt();
        let drt_txid = drt_tx.compute_txid();

        let takeback_hash = TapNodeHash::from_byte_array([0x22; 32]);
        let ee_address = vec![0x11; 20];

        let operator_keys = create_test_operator_keys();
        let agg_pubkey = operator_keys[0].x_only_public_key(SECP256K1).0;

        let magic_bytes = b"TEST";
        let bridge_out_amount = Amount::from_sat(1_000_000_000);

        let tx = build_deposit_transaction(
            drt_txid,
            0,
            ee_address,
            takeback_hash,
            magic_bytes,
            bridge_out_amount,
            agg_pubkey,
        )
        .unwrap();

        // Verify structure
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output.len(), 2);
        assert!(tx.output[0].script_pubkey.is_op_return());
        assert_eq!(tx.output[1].value, bridge_out_amount);
        assert_eq!(tx.input[0].previous_output.txid, drt_txid);
        assert_eq!(tx.input[0].previous_output.vout, 0);
    }
}
