use strata_asm_common::{AsmSpec, Stage};
use strata_asm_proto_bridge_v1::subprotocol::BridgeV1Subproto;
use strata_asm_proto_core::OLCoreSubproto;
use strata_l1_txfmt::MagicBytes;

/// ASM spec for the Strata protocol.
#[derive(Debug)]
pub struct StrataAsmSpec;

impl AsmSpec for StrataAsmSpec {
    const MAGIC_BYTES: MagicBytes = *b"ALPN";

    fn call_subprotocols(stage: &mut impl Stage) {
        stage.process_subprotocol::<OLCoreSubproto>();
        stage.process_subprotocol::<BridgeV1Subproto>();
    }
}
