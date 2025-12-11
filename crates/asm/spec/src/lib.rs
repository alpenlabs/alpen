//! # Strata ASM Specification
//!
//! This crate provides the Anchor State Machine (ASM) specification for the Strata protocol.
//! The ASM specification defines which subprotocols are enabled, their genesis configurations,
//! and protocol-level parameters like magic bytes.

use strata_asm_common::{AsmSpec, Loader, Stage};
use strata_asm_proto_bridge_v1::{BridgeV1Config, BridgeV1Subproto};
use strata_asm_proto_checkpoint::{CheckpointConfig, CheckpointSubprotocol};
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
    checkpoint_config: CheckpointConfig,
    bridge_v1_genesis: BridgeV1Config,
}

impl AsmSpec for StrataAsmSpec {
    fn magic_bytes(&self) -> MagicBytes {
        self.magic_bytes
    }

    fn load_subprotocols(&self, loader: &mut impl Loader) {
        // TODO avoid clone?
        loader.load_subprotocol::<CheckpointSubprotocol>(self.checkpoint_config.clone());
        loader.load_subprotocol::<BridgeV1Subproto>(self.bridge_v1_genesis.clone());
    }

    fn call_subprotocols(&self, stage: &mut impl Stage) {
        stage.invoke_subprotocol::<CheckpointSubprotocol>();
        stage.invoke_subprotocol::<BridgeV1Subproto>();
    }
}

impl StrataAsmSpec {
    /// Creates a new ASM spec instance.
    pub fn new(
        magic_bytes: strata_l1_txfmt::MagicBytes,
        checkpoint_config: CheckpointConfig,
        bridge_v1_genesis: BridgeV1Config,
    ) -> Self {
        Self {
            magic_bytes,
            checkpoint_config,
            bridge_v1_genesis,
        }
    }

    pub fn from_params(params: &RollupParams) -> Self {
        let OperatorConfig::Static(operators) = params.operator_config.clone();

        let checkpoint_config = CheckpointConfig {
            sequencer_cred: params.cred_rule.clone(),
            checkpoint_predicate: params.checkpoint_predicate.clone(),
            genesis_l1_block: params.genesis_l1_view.blk,
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

        Self {
            magic_bytes: params.magic_bytes,
            checkpoint_config,
            bridge_v1_genesis,
        }
    }
}
