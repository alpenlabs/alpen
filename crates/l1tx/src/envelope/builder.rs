use bitcoin::{
    blockdata::script,
    opcodes::{
        all::{OP_ENDIF, OP_IF},
        OP_FALSE,
    },
    script::PushBytesBuf,
    ScriptBuf,
};
use strata_params::Params;
use strata_primitives::l1::payload::{L1Payload, L1PayloadType};

// Generates a [`ScriptBuf`] that consists of `OP_IF .. OP_ENDIF` block
pub fn build_envelope_script(params: &Params, payloads: &[L1Payload]) -> anyhow::Result<ScriptBuf> {
    let mut bytes = Vec::new();
    for payload in payloads {
        let script_bytes = build_payload_envelope(params, payload)?;
        bytes.extend(script_bytes);
    }
    Ok(ScriptBuf::from_bytes(bytes))
}

fn build_payload_envelope(params: &Params, payload: &L1Payload) -> anyhow::Result<Vec<u8>> {
    let type_bytes = get_payload_type_tag(payload.payload_type(), params);
    let type_tag = PushBytesBuf::try_from(type_bytes)?;
    let mut builder = script::Builder::new()
        .push_opcode(OP_FALSE)
        .push_opcode(OP_IF)
        .push_slice(type_tag);

    // Insert actual data
    for chunk in payload.data().chunks(520) {
        builder = builder.push_slice(PushBytesBuf::try_from(chunk.to_vec())?);
    }
    builder = builder.push_opcode(OP_ENDIF);
    Ok(builder.as_bytes().to_vec())
}

fn get_payload_type_tag(payload_type: &L1PayloadType, params: &Params) -> Vec<u8> {
    match *payload_type {
        L1PayloadType::Checkpoint => params.rollup().checkpoint_tag.as_bytes().to_vec(),
        L1PayloadType::Da => params.rollup().da_tag.as_bytes().to_vec(),
    }
}
