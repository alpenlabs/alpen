use bitcoin::{
    ScriptBuf, XOnlyPublicKey,
    opcodes::all::{OP_CHECKSIGVERIFY, OP_CSV},
    script::Builder,
    secp256k1::Secp256k1,
    taproot::TaprootBuilder,
};

pub fn create_takeback_taproot_output(
    recovery_pk: &[u8; 32],
    internal_key: XOnlyPublicKey,
    recovery_delay: u32,
) -> ScriptBuf {
    let secp = Secp256k1::new();

    let tapscript = Builder::new()
        .push_slice(recovery_pk)
        .push_opcode(OP_CHECKSIGVERIFY)
        .push_int(recovery_delay as i64)
        .push_opcode(OP_CSV)
        .into_script();

    let taproot_builder = TaprootBuilder::new()
        .add_leaf(0, tapscript)
        .expect("valid tapscript leaf");
    let spend_info = taproot_builder
        .finalize(&secp, internal_key)
        .expect("taproot finalization should succeed");

    ScriptBuf::new_p2tr(&secp, internal_key, spend_info.merkle_root())
}
