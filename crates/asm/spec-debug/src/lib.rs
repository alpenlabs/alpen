//! # Debug ASM Specification
//!
//! This crate provides the Debug ASM specification for the Strata protocol.
//! The Debug ASM spec wraps the regular ASM spec and adds debug capabilities for testing.
//!
//! **Security Note**: This spec should only be used in testing environments.

use strata_asm_common::{AsmSpec, Loader, Stage};
use strata_asm_params::AdministrationSubprotoParams;
use strata_asm_proto_bridge_v1::BridgeV1Config;
use strata_asm_proto_checkpoint_v0::CheckpointV0Params;
use strata_asm_proto_debug_v1::DebugSubproto;
use strata_asm_spec::StrataAsmSpec;
use strata_l1_txfmt::MagicBytes;

/// Debug ASM specification that includes the debug subprotocol.
///
/// This specification wraps the regular ASM spec and adds debug capabilities for testing.
/// It delegates most functionality to the wrapped production spec but adds the debug subprotocol
/// to the processing pipeline.
///
/// **Security Note**: This spec should only be used in testing environments.
#[derive(Debug)]
pub struct DebugAsmSpec {
    /// The wrapped production ASM spec
    inner: StrataAsmSpec,
}

impl AsmSpec for DebugAsmSpec {
    fn magic_bytes(&self) -> MagicBytes {
        self.inner.magic_bytes()
    }

    fn load_subprotocols(&self, loader: &mut impl Loader) {
        // Load debug subprotocol first
        loader.load_subprotocol::<DebugSubproto>(());

        // Then load all production subprotocols
        self.inner.load_subprotocols(loader);
    }

    fn call_subprotocols(&self, stage: &mut impl Stage) {
        // Call debug subprotocol first
        stage.invoke_subprotocol::<DebugSubproto>();

        // Then call all production subprotocols
        self.inner.call_subprotocols(stage);
    }
}

impl DebugAsmSpec {
    /// Creates a debug ASM spec by wrapping a production spec.
    ///
    /// This adds debug capabilities to an existing production spec.
    pub fn new(
        magic_bytes: strata_l1_txfmt::MagicBytes,
        checkpoint_v0_params: CheckpointV0Params,
        bridge_v1_genesis: BridgeV1Config,
        admin_params: AdministrationSubprotoParams,
    ) -> Self {
        let inner = StrataAsmSpec::new(
            magic_bytes,
            checkpoint_v0_params,
            bridge_v1_genesis,
            admin_params,
        );
        Self { inner }
    }
}
