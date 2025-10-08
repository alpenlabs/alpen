use bitcoin::{
    opcodes::all::{OP_ENDIF, OP_IF},
    script::{Instruction, Instructions},
    ScriptBuf,
};
use thiserror::Error;

use crate::utils::next_op;

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
/// FIXME:PG update docstring
pub fn parse_envelope_payload(script: &ScriptBuf) -> Result<Vec<u8>, EnvelopeParseError> {
    let mut instructions = script.instructions();

    enter_envelope(&mut instructions)?;

    // Parse payload
    let payload = extract_until_op_endif(&mut instructions)?;
    Ok(payload)
}

/// Check for consecutive `OP_FALSE` and `OP_IF` that marks the beginning of an envelope
pub fn enter_envelope(instructions: &mut Instructions<'_>) -> Result<(), EnvelopeParseError> {
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
pub fn extract_until_op_endif(
    instructions: &mut Instructions<'_>,
) -> Result<Vec<u8>, EnvelopeParseError> {
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

    use strata_primitives::l1::payload::L1Payload;

    use super::*;
    use crate::envelope::builder::build_envelope_script;

    #[test]
    fn test_parse_envelope_data() {
        let bytes = vec![0, 1, 2, 3];
        let small_envelope = L1Payload::new_checkpoint(bytes.clone());
        let script = build_envelope_script(&small_envelope).unwrap();
        let result = parse_envelope_payload(&script).unwrap();

        assert_eq!(result, bytes);

        // Try with larger size
        let bytes = vec![1; 2000];
        let large_envelope = L1Payload::new_checkpoint(bytes.clone());
        let script = build_envelope_script(&large_envelope).unwrap();

        let result = parse_envelope_payload(&script).unwrap();
        assert_eq!(result, bytes);
    }
}
