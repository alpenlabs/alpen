//! The `asm_stf` crate implements the core Anchor State Machine state transition function (STF). It
//! glues together block‐level validation, a set of pluggable subprotocols, and the global chain
//! view into a single deterministic state transition.

use std::collections::BTreeMap;

use bitcoin::{block::Block, params::Params};
use strata_asm_common::{AnchorState, AsmError, AsmResult, AsmSpec, GenesisConfigRegistry};

use crate::{
    manager::{AnchorStateLoader, SubprotoManager},
    stage::PreProcessStage,
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
/// Returns an `AsmResult` containing:
/// - `AsmPreProcessOutput` with filtered transactions and auxiliary requests on success
/// - `AsmError` if validation fails or pre-processing encounters an error
///
/// # Errors
///
/// This function will return an error if:
/// - The block header fails PoW continuity validation
/// - Subprotocol loading or pre-processing fails
///
/// # Type Parameters
///
/// * `S` - The ASM specification type that defines magic bytes, subprotocol behavior, and genesis
///   configs
/// * `'b` - Lifetime parameter tied to the input block reference
pub fn pre_process_asm<'b, S: AsmSpec>(
    spec: &S,
    pre_state: &AnchorState,
    block: &'b Block,
) -> AsmResult<AsmPreProcessOutput<'b>> {
    // 1. Validate and update PoW header continuity for the new block.
    // This ensures the block header follows proper Bitcoin consensus rules and chain continuity.
    let mut pow_state = pre_state.chain_view.pow_state.clone();
    pow_state
        .check_and_update(&block.header)
        .map_err(AsmError::InvalidL1Header)?;

    // 2. Filter and group transactions by subprotocol based on magic bytes.
    // Only transactions relevant to registered subprotocols are processed further.
    let grouped_relevant_txs = group_txs_by_subprotocol(spec.magic_bytes(), &block.txdata);

    let mut manager = SubprotoManager::new();

    // 3. LOAD: Initialize each subprotocol in the subproto manager.
    // We use empty aux_payload in the loader stage as no auxiliary data is needed during loading.
    // FIXME ^this should have been a red flag about the aux interface
    let aux = BTreeMap::new();
    let mut loader = AnchorStateLoader::new(pre_state, &mut manager, &aux);
    spec.load_subprotocols(&mut loader);

    // 4. PROCESS: Feed each subprotocol its filtered transactions for pre-processing.
    // This stage extracts auxiliary requests that will be needed for the main STF execution.
    let mut pre_process_stage =
        PreProcessStage::new(&grouped_relevant_txs, &mut manager, pre_state);
    spec.call_subprotocols(&mut pre_process_stage);

    // 5. Flatten the grouped transactions back into a single collection.
    // The grouping was needed for per-subprotocol processing, but the output needs a flat list.
    let relevant_txs: Vec<_> = grouped_relevant_txs.into_values().flatten().collect();

    // 6. Export auxiliary requests collected during pre-processing.
    // These requests will be fulfilled before running the main ASM state transition.
    let aux_requests = manager.export_aux_requests();
    let output = AsmPreProcessOutput {
        txs: relevant_txs,
        aux_requests,
    };

    Ok(output)
}
