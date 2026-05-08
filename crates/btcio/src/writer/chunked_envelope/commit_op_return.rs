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
// NOTE: scanners derive the reveal chunk count from the run of consecutive
// P2TR outputs after the OP_RETURN. Sound only because change uses a
// non-P2TR script (P2WPKH). To allow P2TR change, either encode the chunk
// count in the commit OP_RETURN payload (with a parser helper), or move the
// OP_RETURN to sit just before change so it acts as the reveal-range
// delimiter.
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
