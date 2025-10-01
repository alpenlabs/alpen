use std::io::{self, Read};

use bitcoin::{
    ScriptBuf,
    opcodes::all::OP_ENDIF,
    script::{Instruction, Instructions},
};
use borsh::BorshDeserialize;
use strata_asm_common::TxInputRef;
use strata_l1tx::envelope::parser::enter_envelope;
use strata_ol_chainstate_types::Chainstate;
use strata_primitives::batch::{Checkpoint, SignedCheckpoint};
use strata_state::bridge_ops::WithdrawalIntent;

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

    parse_envelope_payload(&payload_script)
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

fn parse_envelope_payload(script: &ScriptBuf) -> CheckpointTxResult<SignedCheckpoint> {
    let mut instructions = script.instructions();

    enter_envelope(&mut instructions).map_err(CheckpointTxError::EnvelopeParse)?;

    let mut reader = EnvelopePayloadReader::new(instructions);

    let checkpoint = SignedCheckpoint::deserialize_reader(&mut reader).map_err(|err| {
        if matches!(
            err.kind(),
            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
        ) {
            CheckpointTxError::EnvelopeIo(err)
        } else {
            CheckpointTxError::Deserialization(err)
        }
    })?;

    reader.finish().map_err(CheckpointTxError::EnvelopeIo)?;

    Ok(checkpoint)
}

struct EnvelopePayloadReader<'a> {
    instructions: Instructions<'a>,
    current: &'a [u8],
    offset: usize,
    finished: bool,
}

impl<'a> EnvelopePayloadReader<'a> {
    fn new(instructions: Instructions<'a>) -> Self {
        Self {
            instructions,
            current: &[],
            offset: 0,
            finished: false,
        }
    }

    fn advance_chunk(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }

        for next in self.instructions.by_ref() {
            let instruction =
                next.map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            match instruction {
                Instruction::PushBytes(bytes) => {
                    let data = bytes.as_bytes();
                    if data.is_empty() {
                        // Skip empty pushes and continue searching for meaningful payload bytes.
                        continue;
                    }
                    self.current = data;
                    self.offset = 0;
                    return Ok(());
                }
                Instruction::Op(op) if op == OP_ENDIF => {
                    self.finished = true;
                    self.current = &[];
                    self.offset = 0;
                    return Ok(());
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid opcode in checkpoint envelope payload",
                    ));
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "checkpoint envelope terminated without OP_ENDIF",
        ))
    }

    fn finish(&mut self) -> io::Result<()> {
        if self.offset < self.current.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "checkpoint envelope contained trailing payload bytes",
            ));
        }

        if self.finished {
            return Ok(());
        }

        self.advance_chunk()?;

        if self.finished && self.current.is_empty() && self.offset == 0 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "checkpoint envelope contained trailing opcodes",
            ))
        }
    }
}

impl<'a> Read for EnvelopePayloadReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        loop {
            if self.offset < self.current.len() {
                let remaining = &self.current[self.offset..];
                let to_copy = remaining.len().min(buf.len());
                buf[..to_copy].copy_from_slice(&remaining[..to_copy]);
                self.offset += to_copy;
                return Ok(to_copy);
            }

            if self.finished {
                return Ok(0);
            }

            self.advance_chunk()?;

            if self.finished && self.current.is_empty() {
                return Ok(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use bitcoin::{
        ScriptBuf,
        opcodes::all::OP_IF,
        script::{Builder, PushBytesBuf},
    };
    use strata_test_utils::ArbitraryGenerator;

    use super::*;

    #[test]
    fn parse_envelope_payload_streams_signed_checkpoint() {
        let mut arb = ArbitraryGenerator::new();
        let signed_checkpoint: SignedCheckpoint = arb.generate();
        let serialized = borsh::to_vec(&signed_checkpoint).expect("serialize checkpoint");

        // Split the serialized bytes across multiple pushes and include empty pushes to ensure
        // the reader skips them without allocating an intermediate buffer.
        let split = serialized
            .len()
            .saturating_div(2)
            .max(1)
            .min(serialized.len());
        let (first_half, second_half) = serialized.split_at(split);

        let script: ScriptBuf = Builder::new()
            .push_slice(PushBytesBuf::new())
            .push_opcode(OP_IF)
            .push_slice(PushBytesBuf::try_from(first_half.to_vec()).expect("first chunk"))
            .push_slice(PushBytesBuf::new())
            .push_slice(PushBytesBuf::try_from(second_half.to_vec()).expect("second chunk"))
            .push_opcode(OP_ENDIF)
            .into_script();

        let parsed = parse_envelope_payload(&script).expect("parse checkpoint payload");
        assert_eq!(parsed, signed_checkpoint);
    }

    #[test]
    fn parse_envelope_payload_errors_without_closing_endif() {
        let mut arb = ArbitraryGenerator::new();
        let checkpoint: SignedCheckpoint = arb.generate();
        let serialized = borsh::to_vec(&checkpoint).expect("serialize checkpoint");

        // Build a malformed envelope lacking OP_ENDIF so the reader surfaces an UnexpectedEof.
        let script: ScriptBuf = Builder::new()
            .push_slice(PushBytesBuf::new())
            .push_opcode(OP_IF)
            .push_slice(
                PushBytesBuf::try_from(serialized.clone()).expect("malformed payload bytes"),
            )
            .into_script();

        let err = parse_envelope_payload(&script).expect_err("expected envelope parse error");
        assert!(matches!(err, CheckpointTxError::EnvelopeIo(_)));
    }
}
