//! # Strata ASM Specification
//!
//! This crate provides the Anchor State Machine (ASM) specification for the Strata protocol.
//! The ASM specification defines which subprotocols are enabled, their genesis configurations,
//! and protocol-level parameters like magic bytes.

use strata_asm_common::{AsmSpec, GenesisProvider, Stage};
use strata_asm_proto_bridge_v1::{BridgeV1Config, BridgeV1Subproto};
use strata_asm_proto_core::{CoreGenesisConfig, OLCoreSubproto};
use strata_l1_txfmt::MagicBytes;

/// ASM specification for the Strata protocol.
///
/// Implements the [`AsmSpec`] trait to define subprotocol processing order,
/// magic bytes for L1 transaction filtering, and genesis configurations.
#[derive(Debug)]
pub struct StrataAsmSpec {
    magic_bytes: MagicBytes,
    core_genesis: CoreGenesisConfig,
    bridge_v1_genesis: BridgeV1Config,
}

impl AsmSpec for StrataAsmSpec {
    fn magic_bytes(&self) -> MagicBytes {
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

impl GenesisProvider<OLCoreSubproto> for StrataAsmSpec {
    fn genesis_config(&self) -> &CoreGenesisConfig {
        &self.core_genesis
    }
}

impl GenesisProvider<BridgeV1Subproto> for StrataAsmSpec {
    fn genesis_config(&self) -> &BridgeV1Config {
        &self.bridge_v1_genesis
    }
}

impl StrataAsmSpec {
    /// Creates a new ASM specification.
    pub fn new(
        magic_bytes: strata_l1_txfmt::MagicBytes,
        core_genesis: CoreGenesisConfig,
        bridge_v1_genesis: BridgeV1Config,
    ) -> Self {
        Self {
            magic_bytes,
            core_genesis,
            bridge_v1_genesis,
        }
    }
}
