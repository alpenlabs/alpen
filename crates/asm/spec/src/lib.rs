//! # Strata ASM Specification
//!
//! This crate provides the Anchor State Machine (ASM) specification for the Strata protocol.
//! The ASM specification defines which subprotocols are enabled, their genesis configurations,
//! and protocol-level parameters like magic bytes.

use strata_asm_common::{AsmSpec, Loader, Stage};
use strata_asm_proto_bridge_v1::{BridgeV1Config, BridgeV1Subproto};
use strata_asm_proto_checkpoint::{CheckpointConfig, CheckpointSubprotocol};
use strata_identifiers::CredRule;
use strata_l1_txfmt::MagicBytes;
use strata_ol_chainstate_types::compute_genesis_ol_state_root;
use strata_params::{OperatorConfig, RollupParams};
use strata_predicate::{PredicateKey, PredicateTypeId};
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
    bridge_v1_genesis: BridgeV1Config,
    checkpoint_genesis: CheckpointConfig,
}

impl AsmSpec for StrataAsmSpec {
    fn magic_bytes(&self) -> MagicBytes {
        self.magic_bytes
    }

    fn load_subprotocols(&self, loader: &mut impl Loader) {
        // TODO avoid clone?
        loader.load_subprotocol::<CheckpointSubprotocol>(self.checkpoint_genesis.clone());
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
        bridge_v1_genesis: BridgeV1Config,
        checkpoint_genesis: CheckpointConfig,
    ) -> Self {
        Self {
            magic_bytes,
            bridge_v1_genesis,
            checkpoint_genesis,
        }
    }

    /// Creates an ASM spec from rollup parameters.
    ///
    /// The genesis OL state root is computed deterministically from the rollup
    /// parameters, enabling proper validation of the first checkpoint's pre-state root.
    pub fn from_params(params: &RollupParams) -> Self {
        let OperatorConfig::Static(operators) = params.operator_config.clone();

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

        let genesis_l1_blk = &params.genesis_l1_view.blk;
        let sequencer_predicate = cred_rule_to_predicate(&params.cred_rule);

        // Compute the genesis OL state root deterministically from rollup params.
        let genesis_ol_state_root = compute_genesis_ol_state_root(
            *genesis_l1_blk,
            params.evm_genesis_block_hash,
            params.evm_genesis_block_state_root,
        );

        let checkpoint_genesis = CheckpointConfig {
            sequencer_predicate,
            checkpoint_predicate: params.checkpoint_predicate.clone(),
            genesis_l1: *genesis_l1_blk,
            genesis_ol_state_root,
        };

        Self {
            magic_bytes: params.magic_bytes,
            bridge_v1_genesis,
            checkpoint_genesis,
        }
    }
}

/// Convert a `CredRule` to a `PredicateKey` for sequencer signature verification.
fn cred_rule_to_predicate(cred_rule: &CredRule) -> PredicateKey {
    match cred_rule {
        CredRule::Unchecked => PredicateKey::always_accept(),
        CredRule::SchnorrKey(pubkey) => {
            PredicateKey::new(PredicateTypeId::Bip340Schnorr, pubkey.as_ref().to_vec())
        }
    }
}
