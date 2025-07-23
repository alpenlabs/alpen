//! The `asm_stf` crate implements the core Anchor State Machine state transition function (STF). It
//! glues together block‐level validation, a set of pluggable subprotocols, and the global chain
//! view into a single deterministic state transition.

use std::collections::BTreeMap;

use bitcoin::{block::Block, params::Params};
use strata_asm_common::{AnchorState, AsmError, AsmResult, AsmSpec};

use crate::{
    manager::SubprotoManager,
    stage::{PreProcessStage, SubprotoLoaderStage},
    tx_filter::group_txs_by_subprotocol,
    types::AsmPreProcessOutput,
};

/// Pre-processes a Bitcoin block for the Anchor State Machine (ASM) state transition.
///
/// This function performs the initial phase of ASM processing, which includes:
///
/// 1. **Block Header Validation**: Verifies Bitcoin consensus rules and chain continuity
/// 2. **Transaction Filtering**: Groups relevant transactions by their target subprotocols
/// 3. **Subprotocol Loading**: Initializes subprotocol states from the anchor state
/// 4. **Auxiliary Input Collection**: Gathers external data requirements from subprotocols
///
/// The output contains all the information needed for the main ASM state transition,
/// including grouped transactions and auxiliary input requests that must be fulfilled
/// before processing can continue.
///
/// # Arguments
///
/// * `pre_state` - The previous anchor state to transition from
/// * `block` - The new L1 Bitcoin block to process
///
/// # Returns
///
/// * `AsmPreProcessOutput` - Contains grouped transactions and auxiliary input requests
/// * `AsmError` - If block validation fails or subprotocol loading encounters errors
///
/// # Type Parameters
///
/// * `S` - The ASM specification type that defines magic bytes and subprotocol behavior
/// * `'b` - Lifetime parameter tied to the input block reference
pub fn pre_process_asm<'b, S: AsmSpec>(
    pre_state: &AnchorState,
    block: &'b Block,
) -> AsmResult<AsmPreProcessOutput<'b>> {
    // 1. Validate and update PoW header continuity for the new block.
    // This ensures the block header follows proper Bitcoin consensus rules and chain continuity.
    let mut pow_state = pre_state.chain_view.pow_state.clone();
    pow_state
        .check_and_update_continuity(&block.header, &Params::MAINNET)
        .map_err(AsmError::InvalidL1Header)?;

    // 2. Filter and group transactions by subprotocol based on magic bytes.
    // Only transactions relevant to registered subprotocols are processed further.
    let grouped_relevant_txs = group_txs_by_subprotocol(S::MAGIC_BYTES, &block.txdata);

    let mut manager = SubprotoManager::new();

    // 3. LOAD: Initialize each subprotocol in the subproto manager.
    // We use empty auxiliary bundle in the loader stage as no auxiliary data is needed during
    // loading.
    let aux = BTreeMap::new();
    let mut loader_stage = SubprotoLoaderStage::new(pre_state, &mut manager, &aux, None);
    S::call_subprotocols(&mut loader_stage);

    // 4. PRE-PROCESS: Collect auxiliary input requirements from each subprotocol.
    // This stage extracts auxiliary requests that will be needed for the main STF execution.
    let mut pre_process_stage =
        PreProcessStage::new(&grouped_relevant_txs, &mut manager, pre_state);
    S::call_subprotocols(&mut pre_process_stage);

    // 5. Extract results and prepare output for main STF processing.
    // Flatten the grouped transactions into a single list while preserving order.
    let relevant_txs: Vec<_> = grouped_relevant_txs.into_values().flatten().collect();

    // Export auxiliary input requests that must be fulfilled before main processing.
    // These requests will be fulfilled before running the main ASM state transition.
    let aux_requests = manager.export_aux_requests();
    let output = AsmPreProcessOutput {
        txs: relevant_txs,
        aux_requests,
    };

    Ok(output)
}
