//! Checkpoint transaction parsing for checkpointing v0
//!
//! This module handles parsing of SPS-50 envelope transactions containing
//! checkpoint data.
//!
//! NOTE: This implementation focuses on compatibility with the current checkpoint
//! format while following SPS-62 envelope structure requirements.

use bitcoin::ScriptBuf;
use strata_asm_common::TxInputRef;
use strata_l1tx::envelope::parser::{enter_envelope, extract_until_op_endif};
use strata_primitives::batch::SignedCheckpoint;
use strata_state::chain_state::Chainstate;

use crate::{
    constants::OL_STF_CHECKPOINT_TX_TYPE,
    error::{CheckpointV0Error, CheckpointV0Result},
};

/// Extract signed checkpoint from SPS-50 envelope transaction
///
/// This function follows the same pattern as administration subprotocol parsing:
/// 1. Enforces exactly 1 envelope in the first input (SPS-62 requirement)
/// 2. Extracts taproot leaf script from first input witness
/// 3. Parses envelope payload using strata-l1tx envelope parser
/// 4. Deserializes checkpoint from payload
/// 5. Extracts signer public key from transaction context
///
/// NOTE: This bridges current checkpoint format with SPS-50 envelope structure
pub fn extract_signed_checkpoint_from_envelope(
    tx: &TxInputRef<'_>,
) -> CheckpointV0Result<SignedCheckpoint> {
    let tx_type = tx.tag().tx_type();

    // Validate transaction type
    if tx_type != OL_STF_CHECKPOINT_TX_TYPE {
        return Err(CheckpointV0Error::UnsupportedTxType(format!(
            "Expected checkpoint tx type {}, got {}",
            OL_STF_CHECKPOINT_TX_TYPE, tx_type
        )));
    }

    // SPS-62 requirement: exactly 1 envelope in first input
    let bitcoin_tx = tx.tx();
    if bitcoin_tx.input.is_empty() {
        return Err(CheckpointV0Error::ParsingError(
            "Transaction has no inputs".to_string(),
        ));
    }

    // Extract taproot leaf script from the first input's witness
    // (following administration subprotocol pattern)
    let payload_script = bitcoin_tx.input[0]
        .witness
        .taproot_leaf_script()
        .ok_or_else(|| {
            CheckpointV0Error::ParsingError(
                "No taproot leaf script found in first input witness".to_string(),
            )
        })?
        .script;

    // Parse the envelope payload from the script
    let envelope_payload = parse_envelope_payload(&payload_script.into())?;

    // Deserialize the signed checkpoint from the payload
    let signed_checkpoint: SignedCheckpoint =
        borsh::from_slice(&envelope_payload).map_err(|_| {
            CheckpointV0Error::ParsingError(
                "Failed to deserialize checkpoint from envelope: invalid format".to_string(),
            )
        })?;

    Ok(signed_checkpoint)
}

/// Parse envelope payload from Bitcoin script
fn parse_envelope_payload(script: &ScriptBuf) -> CheckpointV0Result<Vec<u8>> {
    let mut instructions = script.instructions();

    // Enter the envelope structure in the script (same as administration)
    enter_envelope(&mut instructions).map_err(|e| {
        CheckpointV0Error::ParsingError(format!("Failed to enter envelope structure: {}", e))
    })?;

    // Extract all data until the envelope closing opcode (same as administration)
    let payload = extract_until_op_endif(&mut instructions).map_err(|e| {
        CheckpointV0Error::ParsingError(format!("Failed to extract envelope payload: {}", e))
    })?;

    Ok(payload)
}

pub(crate) fn extract_withdrawal_messages(
    checkpoint: &strata_primitives::batch::Checkpoint,
) -> CheckpointV0Result<Vec<strata_state::bridge_ops::WithdrawalIntent>> {
    // Extract withdrawal messages from the checkpoint's sidecars
    let sidecar = checkpoint.sidecar();

    let chain_state: Chainstate = borsh::from_slice(sidecar.chainstate()).map_err(|e| {
        CheckpointV0Error::ParsingError(format!(
            "Failed to deserialize chain state from checkpoint: {}",
            e
        ))
    })?;

    let pending_withdraws = chain_state.pending_withdraws();

    Ok(pending_withdraws.entries().to_vec())
}
