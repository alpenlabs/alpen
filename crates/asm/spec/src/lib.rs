//! # Strata ASM Specification
//!
//! This crate provides the Anchor State Machine (ASM) specification for the Strata protocol.
//! The ASM specification defines which subprotocols are enabled, their genesis configurations,
//! and protocol-level parameters like magic bytes.

use std::num::NonZero;

use bitcoin::secp256k1::{Parity, PublicKey, XOnlyPublicKey};
use strata_asm_common::{AsmSpec, Loader, Stage};
use strata_asm_proto_administration::{AdministrationSubprotoParams, AdministrationSubprotocol};
use strata_asm_proto_bridge_v1::{BridgeV1Config, BridgeV1Subproto};
use strata_asm_proto_checkpoint_v0::{
    CheckpointV0Params, CheckpointV0Subproto, CheckpointV0VerificationParams,
};
use strata_crypto::threshold_signature::{CompressedPublicKey, ThresholdConfig};
use strata_l1_txfmt::MagicBytes;
use strata_params::{OperatorConfig, RollupParams};
use strata_primitives::{crypto::EvenPublicKey, l1::BitcoinAmount};

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

    pub fn from_params(params: &RollupParams) -> Self {
        let OperatorConfig::Static(operators) = params.operator_config.clone();

        let checkpoint_v0_params = CheckpointV0Params {
            verification_params: CheckpointV0VerificationParams {
                genesis_l1_block: params.genesis_l1_view.blk,
                cred_rule: params.cred_rule.clone(),
                predicate: params.checkpoint_predicate.clone(),
            },
        };

        let operators = operators
            .iter()
            .map(|o| EvenPublicKey::try_from(*o.wallet_pk()).unwrap())
            .collect();

        let bridge_v1_genesis = BridgeV1Config {
            operators,
            denomination: BitcoinAmount::from_sat(params.deposit_amount.to_sat()),
            assignment_duration: params.dispatch_assignment_dur as u64,
            // TODO(QQ): adjust
            operator_fee: BitcoinAmount::ZERO,
        };

        // For now, use the same operator config for admin roles
        // TODO: Add proper admin config to RollupParams
        let OperatorConfig::Static(ref operators) = params.operator_config;
        let admin_pubkeys: Vec<CompressedPublicKey> = operators
            .iter()
            .map(|o| {
                let xonly_bytes = o.wallet_pk();
                let xonly =
                    XOnlyPublicKey::from_slice(xonly_bytes.as_ref()).expect("valid xonly pubkey");
                let pk = PublicKey::from_x_only_public_key(xonly, Parity::Even);
                CompressedPublicKey::from(pk)
            })
            .collect();
        let threshold = NonZero::new(1).unwrap();
        let admin_config = ThresholdConfig::try_new(admin_pubkeys.clone(), threshold).unwrap();

        let admin_params = AdministrationSubprotoParams::new(
            admin_config.clone(),
            admin_config,
            1, // confirmation_depth for queuing updates
        );

        Self {
            magic_bytes: params.magic_bytes,
            checkpoint_v0_params,
            bridge_v1_genesis,
            admin_params,
        }
    }
}
