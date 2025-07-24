//! Deposit Transaction Parser and Validation
//!
//! This module provides functionality for parsing and validating Bitcoin deposit transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Deposit Transaction Structure
//!
//! A deposit transaction is obtained by spending a Deposit Request Transaction (DRT) and has
//! the following structure:
//!
//! ### Inputs
//! - **First Input** (required): Spends a P2TR output from a Deposit Request Transaction
//!   - Contains a witness with a Taproot signature from the aggregated operator key
//!   - The signature proves authorization to create the deposit
//!   - Additional inputs may be present but are ignored
//!
//! ### Outputs
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with:
//!    - Magic number (4 bytes): Protocol instance identifier
//!    - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier
//!    - Transaction type (1 byte): Deposit transaction type
//!    - Auxiliary data (≤74 bytes):
//!      - Deposit index (4 bytes, big-endian u32)
//!      - Tapscript root hash (32 bytes) from the spent DRT
//!      - Destination address (variable length)
//!
//! 2. **P2TR Deposit Output (Index 1)** (required): The actual deposit containing:
//!    - Pay-to-Taproot script with aggregated operator key as internal key
//!    - No merkle root (key-spend only)
//!    - The deposited Bitcoin amount
//!
//! Additional outputs may be present but are ignored during validation.
//!
//! ## Security Model
//!
//! The tapscript root hash from the DRT is critical for maintaining the bridge's security
//! guarantees. It ensures that only properly authorized deposits (with presigned withdrawal
//! transactions) can mint tokens, preserving the 1-of-N trust assumption for withdrawals.

use bitcoin::{
    Amount, OutPoint, ScriptBuf, TapNodeHash, Transaction, TxOut, XOnlyPublicKey,
    hashes::Hash,
    key::TapTweak,
    sighash::{Prevouts, SighashCache},
    taproot::{self, TAPROOT_CONTROL_NODE_SIZE},
};
use secp256k1::Message;
use strata_asm_common::TxInputRef;
use strata_primitives::{
    buf::Buf32,
    l1::{BitcoinAmount, OutputRef, XOnlyPk},
};

use crate::{constants::DEPOSIT_TX_TYPE, errors::DepositError};

/// Length of the deposit index field in the auxiliary data (4 bytes for u32)
const DEPOSIT_IDX_LEN: usize = size_of::<u32>();
/// Length of the tapscript root hash in the auxiliary data (32 bytes)
const TAPSCRIPT_ROOT_LEN: usize = TAPROOT_CONTROL_NODE_SIZE;
/// Minimum length of auxiliary data (fixed fields only, excluding variable destination address)
const MIN_AUX_DATA_LEN: usize = DEPOSIT_IDX_LEN + TAPSCRIPT_ROOT_LEN;

const DEPOSIT_OUTPUT_INDEX: u32 = 1;

/// Information extracted from a Bitcoin deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositInfo {
    /// The index of the deposit in the bridge's deposit table.
    pub deposit_idx: u32,

    /// The amount of Bitcoin deposited.
    pub amt: BitcoinAmount,

    /// The destination address for the deposit.
    pub address: Vec<u8>,

    /// The outpoint of the deposit transaction.
    pub outpoint: OutputRef,

    /// The tapnode hash (merkle root) from the Deposit Request Transaction (DRT) being spent.
    ///
    /// This value is extracted from the auxiliary data and represents the merkle root of the
    /// tapscript tree from the DRT that this deposit transaction is spending. It is combined
    /// with the internal key (aggregated operator key) to reconstruct the taproot address
    /// that was used in the DRT's P2TR output.
    ///
    /// This is required to verify that the transaction was indeed signed by the claimed pubkey.
    /// Without this validation, someone could send funds to the N-of-N address without proper
    /// authorization, which would mint tokens but break the peg since there would be no presigned
    /// withdrawal transactions. This would require N-of-N trust for withdrawals instead of the
    /// intended 1-of-N trust assumption with presigned transactions.
    pub drt_tapnode_hash: Buf32,
}

/// Extracts deposit information from a Bitcoin bridge deposit transaction.
///
/// Parses a deposit transaction following the SPS-50 specification and extracts
/// the deposit information including amount, destination address, and validation data.
/// See the module-level documentation for the complete transaction structure.
///
/// # Parameters
///
/// - `tx_input` - Reference to the transaction input containing the deposit transaction and its
///   associated tag data
///
/// # Returns
///
/// - `Ok(DepositInfo)` - Successfully parsed deposit information
/// - `Err(DepositError)` - If the transaction structure is invalid, signature verification fails,
///   or any parsing step encounters malformed data
pub fn extract_deposit_info<'a>(tx_input: &TxInputRef<'a>) -> Result<DepositInfo, DepositError> {
    if tx_input.tag().tx_type() != DEPOSIT_TX_TYPE {
        return Err(DepositError::InvalidTxType {
            expected: DEPOSIT_TX_TYPE,
            actual: tx_input.tag().tx_type(),
        });
    }

    let aux_data = tx_input.tag().aux_data();

    // Validate minimum auxiliary data length (must have at least the fixed fields)
    if aux_data.len() < MIN_AUX_DATA_LEN {
        return Err(DepositError::InvalidAuxiliaryData(aux_data.len()));
    }

    // Parse deposit index (bytes 0-3)
    let (deposit_idx_bytes, rest) = aux_data.split_at(DEPOSIT_IDX_LEN);
    let deposit_idx = u32::from_be_bytes(
        deposit_idx_bytes
            .try_into()
            .expect("Expected deposit index to be 4 bytes"),
    );

    // Parse tapscript root hash (bytes 4-35)
    let (tapscript_root_bytes, destination_address) = rest.split_at(TAPSCRIPT_ROOT_LEN);
    let tapscript_root = Buf32::new(
        tapscript_root_bytes
            .try_into()
            .expect("Expected tapscript root to be 32 bytes"),
    );

    // Destination address is remaining bytes (bytes 36+)
    // Must have at least 1 byte for destination address
    if destination_address.is_empty() {
        return Err(DepositError::InvalidAuxiliaryData(aux_data.len()));
    }

    // Extract the deposit output (second output at index 1)
    let deposit_output = tx_input
        .tx()
        .output
        .get(DEPOSIT_OUTPUT_INDEX as usize)
        .ok_or(DepositError::MissingOutput(1))?;

    // Create outpoint reference for the deposit output
    let deposit_outpoint = OutputRef::from(OutPoint {
        txid: tx_input.tx().compute_txid(),
        vout: DEPOSIT_OUTPUT_INDEX,
    });

    // Construct the validated deposit information
    Ok(DepositInfo {
        deposit_idx,
        amt: deposit_output.value.into(),
        address: destination_address.to_vec(),
        outpoint: deposit_outpoint,
        drt_tapnode_hash: tapscript_root,
    })
}

/// Validates that the DRT spending signature in the deposit transaction is valid.
///
/// This function performs Taproot signature validation to verify that the deposit transaction
/// properly spends the Deposit Request Transaction (DRT) with a valid signature from the
/// aggregated operator key.
///
/// The validation process includes:
///
/// 1. **Witness Extraction** - Extracts the signature from the transaction witness
/// 2. **Signature Parsing** - Parses the Taproot signature (supports both 64-byte and 65-byte
///    formats)
/// 3. **Key Derivation** - Derives the tweaked public key from the internal key and merkle root
/// 4. **Sighash Computation** - Computes the transaction sighash for signature verification
/// 5. **Schnorr Verification** - Verifies the Schnorr signature against the tweaked key
///
/// # Parameters
///
/// - `tx` - The deposit transaction that spends the DRT
/// - `drt_tapnode_hash` - The tapscript root hash from the DRT being spent, used to reconstruct the
///   correct taproot address for signature verification
/// - `operators_pubkey` - The aggregated operator public key that should have signed the
///   transaction
/// - `deposit_amount` - The amount from the DRT output being spent
///
/// # Returns
///
/// - `Ok(())` - If the signature is cryptographically valid for the given public key
/// - `Err(DepositError::InvalidSignature)` - If signature validation fails with details about the
///   failure
///
/// # Implementation Details
///
/// Currently uses manual signature verification due to limitations in the bitcoin
/// crate's consensus validation. Future versions should migrate to using the
/// built-in `tx.verify()` method once bitcoinconsensus supports Taproot.
pub fn validate_drt_spending_signature(
    tx: &Transaction,
    drt_tapnode_hash: Buf32,
    operators_pubkey: &XOnlyPk,
    deposit_amount: Amount,
) -> Result<(), DepositError> {
    // Initialize necessary variables and dependencies
    let secp = secp256k1::SECP256K1;

    // FIXME: Use latest version of `bitcoin` once released. The underlying
    // `bitcoinconsensus==0.106` will have support for taproot validation. So here, we just need
    // to create TxOut from operator pubkeys and tapnode hash and call `tx.verify()`.

    // Extract and validate input signature
    let input = tx.input[0].clone();

    // Check if witness is present.
    if input.witness.is_empty() {
        return Err(DepositError::InvalidSignature {
            reason: "No witness data found in transaction input".to_string(),
        });
    }
    let sig_witness = &input.witness[0];

    // rust-bitcoin taproot::Signature handles both both 64-byte (SIGHASH_DEFAULT)
    // and 65-byte (explicit sighash) signatures.
    let taproot_sig = taproot::Signature::from_slice(sig_witness).map_err(|e| {
        DepositError::InvalidSignature {
            reason: format!("Failed to parse taproot signature: {e}"),
        }
    })?;
    let schnorr_sig = taproot_sig.signature;
    let sighash_type = taproot_sig.sighash_type;

    // Parse the internal pubkey and merkle root
    let merkle_root: TapNodeHash = TapNodeHash::from_byte_array(drt_tapnode_hash.0);

    let internal_pubkey = XOnlyPublicKey::from_slice(operators_pubkey.inner().as_bytes()).unwrap();
    let (tweaked_key, _) = internal_pubkey.tap_tweak(secp, Some(merkle_root));

    // Build the scriptPubKey for the UTXO
    let script_pubkey = ScriptBuf::new_p2tr(secp, internal_pubkey, Some(merkle_root));

    let utxos = [TxOut {
        value: deposit_amount,
        script_pubkey,
    }];

    // Compute the sighash
    let prevout = Prevouts::All(&utxos);
    let sighash = SighashCache::new(tx)
        // NOTE: preserving the original sighash_type.
        .taproot_key_spend_signature_hash(0, &prevout, sighash_type)
        .unwrap();

    // Prepare the message for signature verification
    let msg = Message::from_digest(*sighash.as_byte_array());

    // Verify the Schnorr signature
    secp.verify_schnorr(&schnorr_sig, &msg, &tweaked_key.to_x_only_public_key())
        .map_err(|e| DepositError::InvalidSignature {
            reason: format!("Schnorr signature verification failed: {e}"),
        })?;

    Ok(())
}

/// Validates that the deposit output is locked to the N/N aggregated operator key.
///
/// This function verifies that the deposit output at `DEPOSIT_OUTPUT_INDEX` is a P2TR
/// output locked to the provided aggregated operator public key with no merkle root
/// (key-spend only). This ensures the deposited funds can only be spent by the N/N
/// operator set.
///
/// # Parameters
///
/// - `tx` - The deposit transaction to validate
/// - `operators_agg_pubkey` - The aggregated operator public key that should control the deposit
///
/// # Returns
///
/// - `Ok(())` - If the deposit output is properly locked to the operator key
/// - `Err(DepositError)` - If the output is missing, has wrong script type, or wrong key
pub fn validate_deposit_output_lock(
    tx: &Transaction,
    operators_agg_pubkey: &XOnlyPk,
) -> Result<(), DepositError> {
    // Get the deposit output
    let deposit_output = tx
        .output
        .get(DEPOSIT_OUTPUT_INDEX as usize)
        .ok_or(DepositError::MissingOutput(DEPOSIT_OUTPUT_INDEX))?;

    // Extract the internal key from the P2TR script
    let secp = secp256k1::SECP256K1;
    let operators_pubkey = XOnlyPublicKey::from_slice(operators_agg_pubkey.inner().as_bytes())
        .map_err(|_| DepositError::InvalidSignature {
            reason: "Invalid operator public key".to_string(),
        })?;

    // Create expected P2TR script with no merkle root (key-spend only)
    let expected_script = ScriptBuf::new_p2tr(secp, operators_pubkey, None);

    // Verify the deposit output script matches the expected P2TR script
    if deposit_output.script_pubkey != expected_script {
        return Err(DepositError::InvalidSignature {
            reason: "Deposit output is not locked to the aggregated operator key".to_string(),
        });
    }

    Ok(())
}
