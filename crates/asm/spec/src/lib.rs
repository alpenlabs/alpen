//! # Strata ASM Specification
//!
//! This crate provides the Anchor State Machine (ASM) specification for the Strata protocol.
//! The ASM specification defines which subprotocols are enabled, their genesis configurations,
//! and protocol-level parameters like magic bytes.

use strata_asm_common::{AsmSpec, Loader, Stage};
use strata_asm_proto_bridge_v1::{BridgeV1Config, BridgeV1Subproto};
use strata_asm_proto_checkpointing_v0::{CheckpointingV0Config, CheckpointingV0Subproto};
use strata_l1_txfmt::MagicBytes;

/// ASM specification for the Strata protocol.
///
/// Implements the [`AsmSpec`] trait to define subprotocol processing order,
/// magic bytes for L1 transaction filtering, and genesis configurations.
#[derive(Debug)]
pub struct StrataAsmSpec {
    magic_bytes: MagicBytes,

    // subproto params, which right now currently just contain the genesis data
    // TODO rename these
    checkpointing_v0_config: CheckpointingV0Config,
    bridge_v1_genesis: BridgeV1Config,
}

impl AsmSpec for StrataAsmSpec {
    fn magic_bytes(&self) -> MagicBytes {
        self.magic_bytes
    }

    fn load_subprotocols(&self, loader: &mut impl Loader) {
        // TODO avoid clone?
        loader.load_subprotocol::<CheckpointingV0Subproto>(self.checkpointing_v0_config.clone());
        loader.load_subprotocol::<BridgeV1Subproto>(self.bridge_v1_genesis.clone());
    }

    fn call_subprotocols(&self, stage: &mut impl Stage) {
        stage.invoke_subprotocol::<CheckpointingV0Subproto>();
        stage.invoke_subprotocol::<BridgeV1Subproto>();
    }
}

impl StrataAsmSpec {
    /// Creates a new ASM spec instance.
    pub fn new(
        magic_bytes: strata_l1_txfmt::MagicBytes,
        checkpointing_v0_config: CheckpointingV0Config,
        bridge_v1_genesis: BridgeV1Config,
    ) -> Self {
        Self {
            magic_bytes,
            checkpointing_v0_config,
            bridge_v1_genesis,
        }
    }
}
