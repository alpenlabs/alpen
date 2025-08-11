use strata_asm_common::{AsmSpec, GenesisProvider, Stage};
use strata_asm_proto_bridge_v1::{BridgeV1GenesisConfig, BridgeV1Subproto};
use strata_asm_proto_core::{CoreGenesisConfig, OLCoreSubproto};

/// Runtime configuration for the Strata ASM specification.
#[derive(Debug)]
pub struct StrataAsmSpec {
    magic_bytes: strata_l1_txfmt::MagicBytes,
    core_genesis: CoreGenesisConfig,
    bridge_v1_genesis: BridgeV1GenesisConfig,
}

impl AsmSpec for StrataAsmSpec {
    fn magic_bytes(&self) -> strata_l1_txfmt::MagicBytes {
        self.magic_bytes
    }

    fn genesis_config_for<S: strata_asm_common::Subprotocol>(&self) -> &S::GenesisConfig
    where
        Self: GenesisProvider<S>,
    {
        <Self as GenesisProvider<S>>::genesis_config(self)
    }

    fn call_subprotocols(&self, stage: &mut impl Stage<Self>) {
        stage.process_subprotocol::<OLCoreSubproto>(self);
        stage.process_subprotocol::<BridgeV1Subproto>(self);
    }
}

// Implement GenesisProvider for each subprotocol
impl GenesisProvider<OLCoreSubproto> for StrataAsmSpec {
    fn genesis_config(&self) -> &CoreGenesisConfig {
        &self.core_genesis
    }
}

impl GenesisProvider<BridgeV1Subproto> for StrataAsmSpec {
    fn genesis_config(&self) -> &BridgeV1GenesisConfig {
        &self.bridge_v1_genesis
    }
}

impl StrataAsmSpec {
    /// Create a new StrataAsmSpec with specified configurations
    pub fn new(
        magic_bytes: strata_l1_txfmt::MagicBytes,
        core_genesis: CoreGenesisConfig,
        bridge_v1_genesis: BridgeV1GenesisConfig,
    ) -> Self {
        Self {
            magic_bytes,
            core_genesis,
            bridge_v1_genesis,
        }
    }
}
