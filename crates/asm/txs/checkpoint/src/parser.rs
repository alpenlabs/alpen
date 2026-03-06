use bitcoin::ScriptBuf;
use strata_asm_common::TxInputRef;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_l1_envelope_fmt::parser::parse_envelope_container;

use crate::errors::{CheckpointTxError, CheckpointTxResult};

/// Result of extracting a checkpoint payload from an envelope transaction.
///
/// Contains the checkpoint payload and the x-only public key used in the
/// envelope's taproot script. Per SPS-51, the ASM treats the taproot
/// script-spend signature as transitively signing the envelope contents when
/// it recognizes the pubkey as the sequencer's key.
#[derive(Debug)]
pub struct EnvelopeCheckpoint {
    pub payload: CheckpointPayload,
    pub envelope_pubkey: Vec<u8>,
}

/// Extract the checkpoint payload and envelope pubkey from an SPS-50-tagged transaction.
///
/// Performs the following steps:
/// - Unwraps the taproot envelope script from the first input witness.
/// - Parses the envelope container to extract the pubkey and payload chunks.
/// - Decodes the payload from `CodecSsz<CheckpointPayload>` format.
pub fn extract_checkpoint_from_envelope(
    tx: &TxInputRef<'_>,
) -> CheckpointTxResult<EnvelopeCheckpoint> {
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

    let (envelope_pubkey, payloads) = parse_envelope_container(&payload_script)?;

    let raw_payload = payloads
        .into_iter()
        .next()
        .ok_or(CheckpointTxError::MissingPayload)?;

    let checkpoint: CodecSsz<CheckpointPayload> =
        decode_buf_exact(&raw_payload).map_err(CheckpointTxError::CodecDecode)?;

    Ok(EnvelopeCheckpoint {
        payload: checkpoint.into_inner(),
        envelope_pubkey,
    })
}
