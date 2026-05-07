//! EE DA commit-tx OP_RETURN script builder.
//!
//! The commit tx uses output 0 for `OP_RETURN <push8: magic(4) ++ version(4)>`.
//! Each following P2TR output funds one reveal. Reveal tapscripts carry chunk
//! payloads, and chunk ordering follows commit-output ordering.

use bitcoin::{blockdata::script, opcodes::all::OP_RETURN, ScriptBuf};
use strata_l1_txfmt::MagicBytes;

/// Encoded byte length of the OP_RETURN payload.
///
/// Layout:
/// - 4 bytes: EE DA magic
/// - 4 bytes: DA blob version
pub(crate) const COMMIT_OP_RETURN_PAYLOAD_LEN: usize = 8;

/// Builds the commit-tx OP_RETURN script: `OP_RETURN <push8: magic ++ version>`.
pub(crate) fn build_commit_op_return(magic: &MagicBytes, version: u32) -> ScriptBuf {
    let mut payload = [0u8; COMMIT_OP_RETURN_PAYLOAD_LEN];
    payload[..4].copy_from_slice(magic.as_bytes());
    payload[4..].copy_from_slice(&version.to_be_bytes());

    script::Builder::new()
        .push_opcode(OP_RETURN)
        .push_slice(payload)
        .into_script()
}
