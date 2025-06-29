//! The `asm_stf` crate implements the core Anchor State Machine state transition function (STF). It
//! glues together block‐level validation, a set of pluggable subprotocols, and the global chain
//! view into a single deterministic state transition.

use bitcoin::{block::Block, params::Params};
use strata_asm_common::{
    AnchorState, AsmError, AsmSpec, ChainViewState, GenesisConfigRegistry, Stage,
};
use strata_asm_proto_bridge_v1::BridgeV1Subproto;
use strata_asm_proto_core::OLCoreSubproto;

use crate::{
    manager::SubprotoManager,
    stage::{FinishStage, ProcessStage, SubprotoLoaderStage},
    tx_filter::group_txs_by_subprotocol,
};

/// ASM spec for the Strata protocol.
#[derive(Debug)]
pub struct StrataAsmSpec;

impl AsmSpec for StrataAsmSpec {
    fn call_subprotocols(stage: &mut impl Stage) {
        stage.process_subprotocol::<OLCoreSubproto>();
        stage.process_subprotocol::<BridgeV1Subproto>();
    }
}

/// Computes the next AnchorState by applying the Anchor State Machine (ASM) state transition
/// function (STF) to the given previous state, new L1 block and optional genesis config registry
/// for subprotocols state initialization.
pub fn asm_stf<S: AsmSpec>(
    pre_state: AnchorState,
    block: &Block,
    genesis_registry: Option<&GenesisConfigRegistry>,
) -> Result<AnchorState, AsmError> {
    // 1. Validate and update PoW header continuity for the new block.
    let mut pow_state = pre_state.chain_view.pow_state.clone();
    pow_state
        .check_and_update_continuity(&block.header, &Params::MAINNET)
        .map_err(AsmError::InvalidL1Header)?;

    // 2. Filter the relevant transactions
    let all_relevant_transactions = group_txs_by_subprotocol(&block.txdata);

    let mut manager = SubprotoManager::new();

    // 3. LOAD: Bring each subprotocol into the subproto manager.
    let mut loader_stage = SubprotoLoaderStage::new(&pre_state, &mut manager, genesis_registry);
    S::call_subprotocols(&mut loader_stage);

    // 4. PROCESS: Feed each subprotocol its slice of txs.
    let mut process_stage = ProcessStage::new(all_relevant_transactions, &mut manager);
    S::call_subprotocols(&mut process_stage);

    // 5. FINISH: Let each subprotocol process its buffered interproto messages.
    let mut finish_stage = FinishStage::new(&mut manager);
    S::call_subprotocols(&mut finish_stage);

    // 6. Construct the final `AnchorState` we return.
    let sections = manager.export_sections();
    let chain_view = ChainViewState { pow_state };
    Ok(AnchorState {
        chain_view,
        sections,
    })
}
