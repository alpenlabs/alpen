use bitcoin::ScriptBuf;
use ssz::Decode;
use strata_asm_common::TxInputRef;
use strata_checkpoint_types_ssz::SignedCheckpointPayload;
use strata_l1_envelope_fmt::parser::parse_envelope_payload;

use crate::errors::{CheckpointTxError, CheckpointTxResult};

/// Extract the signed checkpoint payload from an SPS-50-tagged transaction input.
///
/// Performs the following steps:
/// - Unwraps the taproot envelope script from the first input witness.
/// - Streams the embedded payload directly from the script instructions.
/// - Deserializes the payload into a [`SignedCheckpointPayload`] using SSZ.
pub fn extract_signed_checkpoint_from_envelope(
    tx: &TxInputRef<'_>,
) -> CheckpointTxResult<SignedCheckpointPayload> {
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

    let signed_checkpoint_payload = SignedCheckpointPayload::from_ssz_bytes(&payload)?;

    Ok(signed_checkpoint_payload)
}
