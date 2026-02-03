use rkyv::rancor::Error as RkyvError;
use strata_asm_common::TxInputRef;
use strata_crypto::threshold_signature::SignatureSet;
use strata_l1_envelope_fmt::parser::parse_envelope_payload;

use crate::{actions::MultisigAction, errors::AdministrationTxParseError};

/// A signed administration payload containing both the action and its signatures.
///
/// This structure is serialized with rkyv and embedded in the witness envelope.
/// The OP_RETURN only contains the SPS-50 tag (magic bytes, subprotocol ID, tx type).
#[derive(Clone, Debug, Eq, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SignedPayload {
    /// The administrative action being proposed
    pub action: MultisigAction,
    /// The set of ECDSA signatures authorizing this action
    pub signatures: SignatureSet,
}

impl SignedPayload {
    /// Creates a new signed payload combining an action with its signatures.
    pub fn new(action: MultisigAction, signatures: SignatureSet) -> Self {
        Self { action, signatures }
    }
}

/// Parses a transaction to extract both the multisig action and the signature set.
///
/// This function extracts the signed payload from the taproot leaf script embedded
/// in the transaction's witness data. The payload contains both the administrative
/// action and its authorizing signatures.
///
/// # Arguments
/// * `tx` - A reference to the transaction input to parse
///
/// # Returns
/// A tuple containing:
/// - `MultisigAction` - The administrative action extracted from the envelope payload
/// - `SignatureSet` - The set of indexed ECDSA signatures
///
/// # Errors
/// Returns `AdministrationTxParseError` if:
/// - The transaction lacks a taproot leaf script in its witness
/// - The envelope payload cannot be parsed
/// - The signed payload cannot be deserialized
pub fn parse_tx(
    tx: &TxInputRef<'_>,
) -> Result<(MultisigAction, SignatureSet), AdministrationTxParseError> {
    let tx_type = tx.tag().tx_type();

    // Extract the taproot leaf script from the first input's witness
    let payload_script = tx.tx().input[0]
        .witness
        .taproot_leaf_script()
        .ok_or(AdministrationTxParseError::MalformedTransaction(tx_type))?
        .script;

    // Parse the envelope payload from the script
    let envelope_payload = parse_envelope_payload(&payload_script.into())?;

    // Deserialize the signed payload (action + signatures) from the envelope
    let signed_payload = rkyv::from_bytes::<SignedPayload, RkyvError>(&envelope_payload)
        .map_err(|_| AdministrationTxParseError::MalformedTransaction(tx_type))?;

    Ok((signed_payload.action, signed_payload.signatures))
}
