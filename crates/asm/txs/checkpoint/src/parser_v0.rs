use bitcoin::ScriptBuf;
use rkyv::rancor::Error as RkyvError;
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

    // SAFETY: payload is an owned buffer extracted from the checkpoint envelope and is expected
    // to be produced by our rkyv serializer; we fail fast on malformed data.
    let checkpoint: SignedCheckpoint =
        unsafe { rkyv::from_bytes_unchecked::<SignedCheckpoint, RkyvError>(&payload) }
            .map_err(CheckpointTxError::Deserialization)?;

    Ok(checkpoint)
}

/// Extract withdrawal intents committed inside a checkpoint sidecar.
pub fn extract_withdrawal_messages(
    checkpoint: &Checkpoint,
) -> CheckpointTxResult<Vec<WithdrawalIntent>> {
    let sidecar = checkpoint.sidecar();
    // SAFETY: sidecar.chainstate() returns bytes serialized by our checkpoint builder; the buffer
    // is immutable here and we treat any decode error as a validation failure.
    let chain_state: Chainstate =
        unsafe { rkyv::from_bytes_unchecked::<Chainstate, RkyvError>(sidecar.chainstate()) }
            .map_err(CheckpointTxError::Deserialization)?;

    Ok(chain_state.pending_withdraws().entries().to_vec())
}
