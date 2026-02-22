//! # Strata ASM Specification
//!
//! This crate provides the Anchor State Machine (ASM) specification for the Strata protocol.
//! The ASM specification defines which subprotocols are enabled, their genesis configurations,
//! and protocol-level parameters like magic bytes.

use strata_asm_common::{AsmSpec, Loader, Stage};
use strata_asm_params::{
    AdministrationSubprotoParams, AsmParams, BridgeV1Config, SubprotocolInstance,
};
use strata_asm_proto_administration::AdministrationSubprotocol;
use strata_asm_proto_bridge_v1::BridgeV1Subproto;
use strata_asm_proto_checkpoint_v0::{
    CheckpointV0Params, CheckpointV0Subproto, CheckpointV0VerificationParams,
};
use strata_l1_txfmt::MagicBytes;
use strata_primitives::CredRule;

/// ASM specification for the Strata protocol.
///
/// Implements the [`AsmSpec`] trait to define subprotocol processing order,
/// magic bytes for L1 transaction filtering, and genesis configurations.
#[derive(Debug)]
pub struct StrataAsmSpec {
    magic_bytes: MagicBytes,

    // subproto params, which right now currently just contain the genesis data
    // TODO rename these
    checkpoint_v0_params: CheckpointV0Params,
    bridge_v1_genesis: BridgeV1Config,
    admin_params: AdministrationSubprotoParams,
}

impl AsmSpec for StrataAsmSpec {
    fn magic_bytes(&self) -> MagicBytes {
        self.magic_bytes
    }

    fn load_subprotocols(&self, loader: &mut impl Loader) {
        // TODO avoid clone?
        loader.load_subprotocol::<CheckpointV0Subproto>(self.checkpoint_v0_params.clone());
        loader.load_subprotocol::<BridgeV1Subproto>(self.bridge_v1_genesis.clone());
        loader.load_subprotocol::<AdministrationSubprotocol>(self.admin_params.clone());
    }

    fn call_subprotocols(&self, stage: &mut impl Stage) {
        stage.invoke_subprotocol::<CheckpointV0Subproto>();
        stage.invoke_subprotocol::<BridgeV1Subproto>();
        stage.invoke_subprotocol::<AdministrationSubprotocol>();
    }
}

impl StrataAsmSpec {
    /// Creates a new ASM spec instance.
    pub fn new(
        magic_bytes: strata_l1_txfmt::MagicBytes,
        checkpoint_v0_params: CheckpointV0Params,
        bridge_v1_genesis: BridgeV1Config,
        admin_params: AdministrationSubprotoParams,
    ) -> Self {
        Self {
            magic_bytes,
            checkpoint_v0_params,
            bridge_v1_genesis,
            admin_params,
        }
    }

    pub fn from_asm_params(params: &AsmParams) -> Self {
        let mut checkpoint_config = None;
        let mut bridge_config = None;
        let mut admin_config = None;

        for instance in &params.subprotocols {
            match instance {
                SubprotocolInstance::Checkpoint(cfg) => checkpoint_config = Some(cfg),
                SubprotocolInstance::Bridge(cfg) => bridge_config = Some(cfg),
                SubprotocolInstance::Admin(cfg) => admin_config = Some(cfg),
            }
        }

        let ckpt = checkpoint_config.expect("AsmParams missing Checkpoint subprotocol");
        let checkpoint_v0_params = CheckpointV0Params {
            verification_params: CheckpointV0VerificationParams {
                genesis_l1_block: params.l1_view.blk,
                cred_rule: CredRule::Unchecked, // FIXME: @PG
                predicate: ckpt.checkpoint_predicate.clone(),
            },
        };

        let bridge_v1_genesis = bridge_config
            .expect("AsmParams missing Bridge subprotocol")
            .clone();
        let admin_params = admin_config
            .expect("AsmParams missing Admin subprotocol")
            .clone();

        Self {
            magic_bytes: params.magic,
            checkpoint_v0_params,
            bridge_v1_genesis,
            admin_params,
        }
    }
}
