use bitcoin::{
    opcodes::all::{OP_ENDIF, OP_IF},
    script::{Instruction, Instructions},
    ScriptBuf,
};
use strata_primitives::l1::payload::{L1Payload, L1PayloadType};
use thiserror::Error;
use tracing::*;

use crate::{
    filter::types::TxFilterConfig,
    utils::{next_bytes, next_op},
};

/// Errors that can be generated while parsing envelopes.
#[derive(Debug, Error)]
pub enum EnvelopeParseError {
    /// Does not have an `OP_IF..OP_ENDIF` block
    #[error("Invalid/Missing envelope(NO OP_IF..OP_ENDIF): ")]
    InvalidEnvelope,
    /// Does not have a valid type tag
    #[error("Invalid/Missing type tag")]
    InvalidTypeTag,
    /// Does not have a valid format
    #[error("Invalid Format")]
    InvalidFormat,
    /// Does not have a payload data of expected size
    #[error("Invalid Payload")]
    InvalidPayload,
}

/// Parse [`L1Payload`]
///
/// # Errors
///
/// This function errors if it cannot parse the [`L1Payload`]
pub fn parse_envelope_payloads(
    script: &ScriptBuf,
    filter_conf: &TxFilterConfig,
) -> Result<Vec<L1Payload>, EnvelopeParseError> {
    let mut instructions = script.instructions();

    let mut payloads = Vec::new();
    // TODO: make this sophisticated, i.e. even if one payload parsing fails, continue finding other
    // envelopes and extracting payloads. Or is that really necessary?
    while let Ok(payload) = parse_l1_payload(&mut instructions, filter_conf) {
        payloads.push(payload);
    }
    Ok(payloads)
}

fn parse_l1_payload(
    instructions: &mut Instructions,
    filter_conf: &TxFilterConfig,
) -> Result<L1Payload, EnvelopeParseError> {
    enter_envelope(instructions)?;

    // Parse type
    let ptype = next_bytes(instructions)
        .and_then(|bytes| parse_payload_type(bytes, filter_conf))
        .ok_or(EnvelopeParseError::InvalidTypeTag)?;

    // Parse payload
    let payload = extract_until_op_endif(instructions)?;
    Ok(L1Payload::new(payload, ptype))
}

fn parse_payload_type(tag_bytes: &[u8], filter_conf: &TxFilterConfig) -> Option<L1PayloadType> {
    if filter_conf.envelope_tags.checkpoint_tag.as_bytes() == tag_bytes {
        Some(L1PayloadType::Checkpoint)
    } else if filter_conf.envelope_tags.da_tag.as_bytes() == tag_bytes {
        Some(L1PayloadType::Da)
    } else {
        None
    }
}

/// Check for consecutive `OP_FALSE` and `OP_IF` that marks the beginning of an envelope
fn enter_envelope(instructions: &mut Instructions) -> Result<(), EnvelopeParseError> {
    // loop until OP_FALSE is found
    loop {
        let next = instructions.next();
        match next {
            None => {
                return Err(EnvelopeParseError::InvalidEnvelope);
            }
            // OP_FALSE is basically empty PushBytes
            Some(Ok(Instruction::PushBytes(bytes))) => {
                if bytes.as_bytes().is_empty() {
                    break;
                }
            }
            _ => {
                // Just carry on until OP_FALSE is found
            }
        }
    }

    // Check if next opcode is OP_IF
    let op_if = next_op(instructions);
    if op_if != Some(OP_IF) {
        return Err(EnvelopeParseError::InvalidEnvelope);
    }
    Ok(())
}

/// Extract bytes of `size` from the remaining instructions
fn extract_until_op_endif(instructions: &mut Instructions) -> Result<Vec<u8>, EnvelopeParseError> {
    let mut data = vec![];
    for elem in instructions {
        match elem {
            Ok(Instruction::Op(OP_ENDIF)) => {
                break;
            }
            Ok(Instruction::PushBytes(b)) => {
                data.extend_from_slice(b.as_bytes());
            }
            _ => {
                return Err(EnvelopeParseError::InvalidPayload);
            }
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {

    use strata_btcio::test_utils::generate_envelope_script_test;
    use strata_primitives::l1::payload::L1Payload;
    use strata_test_utils::l2::gen_params;

    use super::*;

    #[test]
    fn test_parse_envelope_data() {
        let bytes = vec![0, 1, 2, 3];
        let params = gen_params();
        let filter_config = TxFilterConfig::derive_from(params.rollup()).unwrap();
        let envelope1 = L1Payload::new_checkpoint(bytes.clone());
        let envelope2 = L1Payload::new_checkpoint(bytes.clone());
        let script =
            generate_envelope_script_test(&[envelope1.clone(), envelope2.clone()], &params)
                .unwrap();
        let result = parse_envelope_payloads(&script, &filter_config).unwrap();

        assert_eq!(result, vec![envelope1, envelope2]);

        // Try with larger size
        let bytes = vec![1; 2000];
        let envelope_data = L1Payload::new_checkpoint(bytes.clone());
        let script =
            generate_envelope_script_test(std::slice::from_ref(&envelope_data), &params).unwrap();

        // Parse the rollup name
        let result = parse_envelope_payloads(&script, &filter_config).unwrap();

        // Assert the rollup name was parsed correctly
        assert_eq!(result, vec![envelope_data]);
    }
}
