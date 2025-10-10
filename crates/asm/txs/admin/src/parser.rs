use bitvec::vec::BitVec;
use strata_asm_common::TxInputRef;
use strata_crypto::multisig::SchnorrMultisigSignature;
use strata_l1_envelope_fmt::parser::parse_envelope_payload;

use crate::{actions::MultisigAction, errors::AdministrationTxParseError};

/// Parses a transaction to extract both the multisig action and the aggregated signature.
///
/// This function extracts the administrative action from the taproot leaf script embedded
/// in the transaction's witness data, and parses the aggregated signature from
/// the transaction's auxiliary data.
///
/// # Arguments
/// * `tx` - A reference to the transaction input to parse
///
/// # Returns
/// A tuple containing:
/// - `MultisigAction` - The administrative action extracted from the envelope payload
/// - `SchnorrMultisigSignature` - The aggregated signature with signer indices
///
/// # Errors
/// Returns `AdministrationTxParseError` if:
/// - The transaction lacks a taproot leaf script in its witness
/// - The envelope payload cannot be parsed
/// - The action cannot be deserialized from the payload
/// - The aggregated signature parsing fails
pub fn parse_tx(
    tx: &TxInputRef<'_>,
) -> Result<(MultisigAction, SchnorrMultisigSignature), AdministrationTxParseError> {
    // Parse the aggregated signature first
    let agg_multisig = parse_aggregated_multisig(tx)?;
    let tx_type = tx.tag().tx_type();

    // Extract the taproot leaf script from the first input's witness
    let payload_script = tx.tx().input[0]
        .witness
        .taproot_leaf_script()
        .ok_or(AdministrationTxParseError::MalformedTransaction(tx_type))?
        .script;

    // Parse the envelope payload from the script
    let envelope_payload = parse_envelope_payload(&payload_script.into())?;

    // Deserialize the multisig action from the payload
    let action = borsh::from_slice(&envelope_payload)
        .map_err(|_| AdministrationTxParseError::MalformedTransaction(tx_type))?;

    Ok((action, agg_multisig))
}

/// Parses the aggregated signature from transaction auxiliary data.
///
/// The auxiliary data contains a 64-byte Schnorr signature followed by a bit vector
/// indicating which signers participated in the aggregated signature.
///
/// # Arguments
/// * `tx` - A reference to the transaction input containing the auxiliary data
///
/// # Returns
/// A `SchnorrMultisigSignature` containing the aggregated signature and signer indices
///
/// # Errors
/// Returns `AdministrationTxParseError` if the auxiliary data format is invalid
///
/// # Data Format
/// The auxiliary data is structured as:
/// - Bytes 0-63: 64-byte Schnorr signature
/// - Bytes 64+: Bit vector representing signer indices
pub fn parse_aggregated_multisig(
    tx: &TxInputRef<'_>,
) -> Result<SchnorrMultisigSignature, AdministrationTxParseError> {
    let data = tx.tag().aux_data();

    // Extract the 64-byte signature from the beginning of aux data
    let mut sig = [0u8; 64];
    sig.copy_from_slice(&data[0..64]);

    // Extract signer indices from the remaining bytes as a bit vector
    let signer_indices_bytes = &data[64..];
    let indices: BitVec<u8> = BitVec::from_slice(signer_indices_bytes);

    Ok(SchnorrMultisigSignature::new(indices, sig.into()))
}
