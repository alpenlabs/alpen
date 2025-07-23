//! The `asm_stf` crate implements the core Anchor State Machine state transition function (STF). It
//! glues together block‐level validation, a set of pluggable subprotocols, and the global chain
//! view into a single deterministic state transition.

use bitcoin::params::Params;
use strata_asm_common::{
    AnchorState, AsmError, AsmResult, AsmSpec, ChainViewState, GenesisConfigRegistry,
};

use crate::{
    manager::SubprotoManager,
    stage::{FinishStage, ProcessStage, SubprotoLoaderStage},
    types::{AsmStfInput, AsmStfOutput},
};

/// Computes the next AnchorState by applying the Anchor State Machine (ASM) state transition
/// function (STF) to the given previous state and new L1 block.
///
/// This function performs the main ASM state transition by validating the block header continuity,
/// loading subprotocols with auxiliary input data, processing protocol-specific transactions,
/// handling inter-protocol communication, and constructing the final state with logs.
///
/// # Arguments
///
/// * `pre_state` - The current anchor state containing chain view and subprotocol states
/// * `input` - The ASM STF input containing the block header, protocol transactions, and auxiliary
///   data
/// * `genesis_registry` - genesis configuration registry for subprotocol initialization
///
/// # Returns
///
/// Returns an `AsmResult` containing:
/// - `AsmStfOutput` with the new anchor state and execution logs on success
/// - `AsmError` if validation fails or state transition encounters an error
///
/// # Errors
///
/// This function will return an error if:
/// - The block header fails PoW continuity validation
/// - Subprotocol loading, processing, or finishing fails
///
/// # Type Parameters
///
/// * `S` - The ASM specification type that defines magic bytes and subprotocol behavior
/// * `'b` - Lifetime parameter tied to the input block reference
/// * `'x` - Lifetime parameter tied to the auxiliary input data
pub fn asm_stf<'b, 'x, S: AsmSpec>(
    pre_state: &AnchorState,
    input: AsmStfInput<'b, 'x>,
    genesis_registry: &GenesisConfigRegistry,
) -> AsmResult<AsmStfOutput> {
    // 1. Validate and update PoW header continuity for the new block.
    // This ensures the block header follows proper Bitcoin consensus rules and chain continuity.
    let mut pow_state = pre_state.chain_view.pow_state.clone();
    pow_state
        .check_and_update_continuity(input.header, &Params::MAINNET)
        .map_err(AsmError::InvalidL1Header)?;

    let mut manager = SubprotoManager::new();

    // 2. LOAD: Initialize each subprotocol in the subproto manager with auxiliary input data and
    // genesis config data if needed.
    let mut loader_stage =
        SubprotoLoaderStage::new(pre_state, &mut manager, input.aux_input, genesis_registry);
    S::call_subprotocols(&mut loader_stage);

    // 3. PROCESS: Feed each subprotocol its filtered transactions for execution.
    // This stage performs the actual state transitions for each subprotocol.
    let mut process_stage = ProcessStage::new(input.protocol_txs, &mut manager, pre_state);
    S::call_subprotocols(&mut process_stage);

    // 4. FINISH: Allow each subprotocol to process buffered inter-protocol messages.
    // This stage handles cross-protocol communication and finalizes state changes.
    let mut finish_stage = FinishStage::new(&mut manager);
    S::call_subprotocols(&mut finish_stage);

    // 5. Construct the final `AnchorState` and output.
    // Export the updated state sections and logs from all subprotocols to build the result.
    let (sections, logs) = manager.export_sections_and_logs();
    let chain_view = ChainViewState { pow_state };
    let state = AnchorState {
        chain_view,
        sections,
    };
    let output = AsmStfOutput { state, logs };
    Ok(output)
}
