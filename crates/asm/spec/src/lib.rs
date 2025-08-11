use strata_asm_common::{AsmSpec, GenesisProvider, Stage};
use strata_asm_proto_bridge_v1::{BridgeV1GenesisConfig, BridgeV1Subproto};
use strata_asm_proto_core::{CoreGenesisConfig, OLCoreSubproto};
use strata_primitives::{buf::Buf32, l1::L1BlockCommitment};
use zkaleido::VerifyingKey;

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

    /// Create a new StrataAsmSpec with default configurations for Strata
    pub fn strata_default() -> Self {
        let core_genesis = CoreGenesisConfig::new(
            VerifyingKey::default(), // TODO: Replace with actual verifying key
            L1BlockCommitment::new(
                0, // TODO: Replace with actual genesis block height
                strata_primitives::l1::L1BlockId::default()
            ), // TODO: Replace with actual genesis L1 block
            Buf32::zero(), // TODO: Replace with actual sequencer pubkey
        ).expect("Failed to create CoreGenesisConfig");

        Self::new(
            *b"ALPN",
            core_genesis,
            BridgeV1GenesisConfig::default(),
        )
    }

    /// Create a new StrataAsmSpec with custom magic bytes but default genesis configs
    pub fn with_magic_bytes(magic_bytes: strata_l1_txfmt::MagicBytes) -> Self {
        let mut spec = Self::strata_default();
        spec.magic_bytes = magic_bytes;
        spec
    }

    /// Update the core genesis configuration
    pub fn set_core_genesis(&mut self, genesis: CoreGenesisConfig) {
        self.core_genesis = genesis;
    }

    /// Update the bridge v1 genesis configuration
    pub fn set_bridge_v1_genesis(&mut self, genesis: BridgeV1GenesisConfig) {
        self.bridge_v1_genesis = genesis;
    }

    /// Update the magic bytes
    pub fn set_magic_bytes(&mut self, magic_bytes: strata_l1_txfmt::MagicBytes) {
        self.magic_bytes = magic_bytes;
    }
}
