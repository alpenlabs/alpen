use bitcoin::{
    blockdata::script,
    opcodes::{
        all::{OP_ENDIF, OP_IF},
        OP_FALSE,
    },
    script::PushBytesBuf,
    ScriptBuf,
};
use strata_primitives::l1::payload::L1Payload;

// Generates a [`ScriptBuf`] that consists of `OP_IF .. OP_ENDIF` block
pub fn build_envelope_script(payload: &L1Payload) -> anyhow::Result<ScriptBuf> {
    let mut builder = script::Builder::new()
        .push_opcode(OP_FALSE)
        .push_opcode(OP_IF);

    // Insert actual data
    for chunk in payload.data().chunks(520) {
        builder = builder.push_slice(PushBytesBuf::try_from(chunk.to_vec())?);
    }
    builder = builder.push_opcode(OP_ENDIF);
    Ok(builder.into_script())
}
