use bitcoin::ScriptBuf;
use strata_asm_common::TxInputRef;
use strata_bridge_types::WithdrawalIntent;
use strata_checkpoint_types::{Checkpoint, SignedCheckpoint};
use strata_l1_envelope_fmt::parser::parse_envelope_payload;
use strata_ol_chainstate_types::Chainstate;

use crate::errors::{CheckpointTxError, CheckpointTxResult};

/// Extract the signed checkpoint payload from an SPS-50-tagged transaction input.
///
/// Performs the following steps:
/// - Unwraps the taproot envelope script from the first input witness.
/// - Streams the embedded payload directly from the script instructions.
/// - Deserializes the payload into a [`SignedCheckpoint`].
pub fn extract_signed_checkpoint_from_envelope(
    tx: &TxInputRef<'_>,
) -> CheckpointTxResult<SignedCheckpoint> {
    let bitcoin_tx = tx.tx();
    if bitcoin_tx.input.is_empty() {
        return Err(CheckpointTxError::MissingInputs);
    }

    let payload_script: ScriptBuf = bitcoin_tx.input[0]
        .witness
        .taproot_leaf_script()
        .ok_or(CheckpointTxError::MissingLeafScript)?
        .script
        .into();

    let payload = parse_envelope_payload(&payload_script)?;

    let checkpoint: SignedCheckpoint =
        borsh::from_slice(&payload).map_err(CheckpointTxError::Deserialization)?;

    Ok(checkpoint)
}

/// Extract withdrawal intents committed inside a checkpoint sidecar.
pub fn extract_withdrawal_messages(
    checkpoint: &Checkpoint,
) -> CheckpointTxResult<Vec<WithdrawalIntent>> {
    let sidecar = checkpoint.sidecar();
    let chain_state: Chainstate =
        borsh::from_slice(sidecar.chainstate()).map_err(CheckpointTxError::Deserialization)?;

    Ok(chain_state.pending_withdraws().entries().to_vec())
}
