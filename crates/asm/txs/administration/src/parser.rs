use bitcoin::ScriptBuf;
use bitvec::vec::BitVec;
use strata_asm_common::TxInputRef;
use strata_crypto::multisig::SchnorrMultisigSignature;
use strata_l1tx::envelope::parser::{enter_envelope, extract_until_op_endif};

use crate::{actions::MultisigAction, errors::AdministrationTxParseError};

pub fn parse_tx_multisig_action_and_vote(
    tx: &TxInputRef<'_>,
) -> Result<(MultisigAction, SchnorrMultisigSignature), AdministrationTxParseError> {
    let vote = parse_aggregated_vote(tx)?;
    let tx_type = tx.tag().tx_type();

    let payload_script = tx.tx().input[0]
        .witness
        .taproot_leaf_script()
        .ok_or(AdministrationTxParseError::MalformedTransaction(tx_type))?
        .script;
    let envelope_payload = parse_envelope_payload(&payload_script.into())?;
    let action = borsh::from_slice(&envelope_payload)
        .map_err(|_| AdministrationTxParseError::MalformedTransaction(tx_type))?;

    Ok((action, vote))
}

/// Extracts the AggregatedVote from a transaction input.
/// FIXME: This is a placeholder function and should be replaced with actual logic.
pub fn parse_aggregated_vote(
    tx: &TxInputRef<'_>,
) -> Result<SchnorrMultisigSignature, AdministrationTxParseError> {
    let data = tx.tag().aux_data();

    let mut sig = [0u8; 64];
    sig.copy_from_slice(&data[0..64]);

    let signer_indices_bytes = &data[64..];
    let indices: BitVec<u8> = BitVec::from_slice(signer_indices_bytes);

    Ok(SchnorrMultisigSignature::new(indices, sig.into()))
}

pub fn parse_envelope_payload(script: &ScriptBuf) -> Result<Vec<u8>, AdministrationTxParseError> {
    let mut instructions = script.instructions();
    enter_envelope(&mut instructions).map_err(AdministrationTxParseError::MalformedEnvelope)?;
    let payload = extract_until_op_endif(&mut instructions)?;
    Ok(payload)
}
