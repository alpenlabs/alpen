use bitcoin::ScriptBuf;
use strata_asm_common::TxInputRef;
use strata_l1tx::envelope::parser::{enter_envelope, extract_until_op_endif};
use strata_ol_chainstate_types::Chainstate;
use strata_primitives::batch::{Checkpoint, SignedCheckpoint};
use strata_state::bridge_ops::WithdrawalIntent;

use crate::{
    constants::OL_STF_CHECKPOINT_TX_TYPE,
    errors::{CheckpointTxError, CheckpointTxResult},
};

/// Extract the signed checkpoint payload from an SPS-50-tagged transaction input.
///
/// Ensures the transaction carries the expected checkpoint tx-type tag, unwraps the taproot
/// envelope script from the first input witness, parses the embedded payload, and deserializes it
/// into a [`SignedCheckpoint`].
pub fn extract_signed_checkpoint_from_envelope(
    tx: &TxInputRef<'_>,
) -> CheckpointTxResult<SignedCheckpoint> {
    let tx_type = tx.tag().tx_type();
    if tx_type != OL_STF_CHECKPOINT_TX_TYPE {
        return Err(CheckpointTxError::UnexpectedTxType {
            expected: OL_STF_CHECKPOINT_TX_TYPE,
            actual: tx_type,
        });
    }

    let bitcoin_tx = tx.tx();
    if bitcoin_tx.input.is_empty() {
        return Err(CheckpointTxError::MissingInputs);
    }

    let payload_script = bitcoin_tx.input[0]
        .witness
        .taproot_leaf_script()
        .ok_or(CheckpointTxError::MissingLeafScript)?
        .script;

    let envelope_payload = parse_envelope_payload(&payload_script.into())?;

    borsh::from_slice(&envelope_payload).map_err(|e| {
        CheckpointTxError::Deserialization(format!("checkpoint payload borsh decode failed: {e}"))
    })
}

/// Extract withdrawal intents committed inside a checkpoint sidecar.
pub fn extract_withdrawal_messages(
    checkpoint: &Checkpoint,
) -> CheckpointTxResult<Vec<WithdrawalIntent>> {
    let sidecar = checkpoint.sidecar();
    let chain_state: Chainstate = borsh::from_slice(sidecar.chainstate()).map_err(|e| {
        CheckpointTxError::Deserialization(format!("checkpoint chainstate decode failed: {e}"))
    })?;

    Ok(chain_state.pending_withdraws().entries().to_vec())
}

fn parse_envelope_payload(script: &ScriptBuf) -> CheckpointTxResult<Vec<u8>> {
    let mut instructions = script.instructions();

    enter_envelope(&mut instructions)
        .map_err(|e| CheckpointTxError::EnvelopeParse(e.to_string()))?;

    extract_until_op_endif(&mut instructions)
        .map_err(|e| CheckpointTxError::EnvelopeParse(e.to_string()))
}
