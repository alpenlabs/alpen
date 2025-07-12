//! The `asm_stf` crate implements the core Anchor State Machine state transition function (STF). It
//! glues together block‐level validation, a set of pluggable subprotocols, and the global chain
//! view into a single deterministic state transition.

use std::collections::BTreeMap;

use bitcoin::{block::Block, params::Params};
use strata_asm_common::{AnchorState, AsmError, AsmResult, AsmSpec, ChainViewState};

use crate::{
    manager::SubprotoManager,
    stage::{FinishStage, PreProcessStage, ProcessStage, SubprotoLoaderStage},
    tx_filter::group_txs_by_subprotocol,
    types::{AsmPreProcessOutput, AsmStfInput, AsmStfOutput},
};

/// Computes the next AnchorState by applying the Anchor State Machine (ASM) state transition
/// function (STF) to the given previous state and new L1 block.
pub fn asm_stf<'b, 'x, S: AsmSpec>(
    pre_state: &AnchorState,
    input: AsmStfInput<'b, 'x>,
) -> AsmResult<AsmStfOutput> {
    // 1. Validate and update PoW header continuity for the new block.
    let mut pow_state = pre_state.chain_view.pow_state.clone();
    pow_state
        .check_and_update_continuity(input.header, &Params::MAINNET)
        .map_err(AsmError::InvalidL1Header)?;

    let mut manager = SubprotoManager::new();

    // 3. LOAD: Bring each subprotocol into the subproto manager.
    let mut loader_stage = SubprotoLoaderStage::new(pre_state, &mut manager, input.aux_input);
    S::call_subprotocols(&mut loader_stage);

    // 4. PROCESS: Feed each subprotocol its slice of txs.
    let mut process_stage = ProcessStage::new(input.protocol_txs, &mut manager, pre_state);
    S::call_subprotocols(&mut process_stage);

    // 5. FINISH: Let each subprotocol process its buffered interproto messages.
    let mut finish_stage = FinishStage::new(&mut manager);
    S::call_subprotocols(&mut finish_stage);

    // 6. Construct the final `AnchorState` we return.
    let (sections, logs) = manager.export_sections_and_logs();
    let chain_view = ChainViewState { pow_state };
    let state = AnchorState {
        chain_view,
        sections,
    };
    let output = AsmStfOutput { state, logs };
    Ok(output)
}

pub fn pre_process_asm<'t, S: AsmSpec>(
    pre_state: &AnchorState,
    block: &'t Block,
) -> AsmResult<AsmPreProcessOutput<'t>> {
    // 1. Validate and update PoW header continuity for the new block.
    let mut pow_state = pre_state.chain_view.pow_state.clone();
    pow_state
        .check_and_update_continuity(&block.header, &Params::MAINNET)
        .map_err(AsmError::InvalidL1Header)?;

    // 2. Filter the relevant transactions
    let grouped_relevant_txs = group_txs_by_subprotocol(S::MAGIC_BYTES, &block.txdata);

    let mut manager = SubprotoManager::new();

    // 3. LOAD: Bring each subprotocol into the subproto manager.
    let aux = BTreeMap::new();
    let mut loader_stage = SubprotoLoaderStage::new(pre_state, &mut manager, &aux);
    S::call_subprotocols(&mut loader_stage);

    // 4. PROCESS: Feed each subprotocol its slice of txs.
    let mut pre_process_stage =
        PreProcessStage::new(&grouped_relevant_txs, &mut manager, pre_state);
    S::call_subprotocols(&mut pre_process_stage);

    let relevant_txs = grouped_relevant_txs
        .into_iter()
        .flat_map(|(_k, vec)| vec)
        .collect();

    let aux_requests = manager.export_aux_requests();
    let output = AsmPreProcessOutput {
        txs: relevant_txs,
        aux_requests,
    };

    Ok(output)
}
