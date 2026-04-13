//! # Strata ASM Specification
//!
//! This crate provides the Anchor State Machine (ASM) specification for the Strata protocol.

use strata_asm_common::{
    AnchorState, AsmHistoryAccumulatorState, AsmSpec, ChainViewState, HeaderVerificationState,
    SectionState, Stage, Subprotocol,
};
use strata_asm_params::AsmParams;
use strata_asm_proto_administration::{AdministrationSubprotoState, AdministrationSubprotocol};
use strata_asm_proto_bridge_v1::{BridgeV1State, BridgeV1Subproto};
use strata_asm_proto_checkpoint::{state::CheckpointState, subprotocol::CheckpointSubprotocol};
use strata_asm_proto_checkpoint_v0::{
    CheckpointV0InitConfig, CheckpointV0Subproto, CheckpointV0VerificationParams,
};
use strata_btc_verification::HeaderVerificationState as NativeHeaderVerificationState;
use strata_params::CredRule;

/// Strata ASM specification.
#[derive(Debug, Default, Clone, Copy)]
pub struct StrataAsmSpec;

impl AsmSpec for StrataAsmSpec {
    type Params = AsmParams;

    fn call_subprotocols(&self, stage: &mut impl Stage) {
        stage.invoke_subprotocol::<AdministrationSubprotocol>();
        stage.invoke_subprotocol::<CheckpointV0Subproto>();
        stage.invoke_subprotocol::<CheckpointSubprotocol>();
        stage.invoke_subprotocol::<BridgeV1Subproto>();
    }

    fn construct_genesis_state(&self, params: &Self::Params) -> AnchorState {
        construct_genesis_state(params)
    }
}

impl StrataAsmSpec {
    /// Creates a new ASM spec instance.
    pub fn new() -> Self {
        Self
    }

    /// Builds the spec from params.
    pub fn from_asm_params(_params: &AsmParams) -> Self {
        Self
    }
}

/// Builds the genesis [`AnchorState`] from the given [`AsmParams`].
pub fn construct_genesis_state(params: &AsmParams) -> AnchorState {
    let genesis_admin_subprotocol_state = AdministrationSubprotoState::new(
        params
            .admin_config()
            .expect("asm: missing Admin subprotocol config in params"),
    );
    let admin_subprotocol_section =
        SectionState::from_state::<AdministrationSubprotocol>(&genesis_admin_subprotocol_state)
            .expect("asm: Admin subprotocol genesis state fits section data capacity");

    let checkpoint_config = params
        .checkpoint_config()
        .expect("asm: missing Checkpoint subprotocol config in params");

    let checkpoint_v0_config = CheckpointV0InitConfig {
        verification_params: CheckpointV0VerificationParams {
            genesis_l1_block: params.anchor.block,
            cred_rule: CredRule::Unchecked,
            predicate: checkpoint_config.checkpoint_predicate.clone(),
        },
    };
    let checkpoint_v0_state = CheckpointV0Subproto::init(&checkpoint_v0_config);
    let checkpoint_v0_section =
        SectionState::from_state::<CheckpointV0Subproto>(&checkpoint_v0_state)
            .expect("asm: Checkpoint-v0 subprotocol genesis state fits section data capacity");

    let checkpoint_state = CheckpointState::init(checkpoint_config.clone());
    let checkpoint_section = SectionState::from_state::<CheckpointSubprotocol>(&checkpoint_state)
        .expect("asm: Checkpoint subprotocol genesis state fits section data capacity");

    let genesis_bridge_subprotocol_state = BridgeV1State::new(
        params
            .bridge_config()
            .expect("asm: missing Bridge subprotocol config in params"),
    );
    let bridge_subprotocol_section =
        SectionState::from_state::<BridgeV1Subproto>(&genesis_bridge_subprotocol_state)
            .expect("asm: Bridge subprotocol genesis state fits section data capacity");

    let native_header_vs = NativeHeaderVerificationState::init(params.anchor.clone());
    let history_accumulator = AsmHistoryAccumulatorState::new(params.anchor.block.height() as u64);
    let chain_view = ChainViewState {
        history_accumulator,
        pow_state: HeaderVerificationState::from_native(native_header_vs),
    };

    AnchorState {
        magic: AnchorState::magic_ssz(params.magic),
        chain_view,
        sections: vec![
            admin_subprotocol_section,
            checkpoint_v0_section,
            checkpoint_section,
            bridge_subprotocol_section,
        ]
        .try_into()
        .expect("asm: genesis sections fit within capacity"),
    }
}
