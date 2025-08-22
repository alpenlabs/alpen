//! The `asm_stf` crate implements the core Anchor State Machine state transition function (STF). It
//! glues together block‐level validation, a set of pluggable subprotocols, and the global chain
//! view into a single deterministic state transition.

use bitcoin::params::Params;
use strata_asm_common::{AnchorState, AsmError, AsmResult, AsmSpec, ChainViewState};

use crate::{
    manager::SubprotoManager,
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
/// * `S` - The ASM specification type that defines magic bytes, subprotocol behavior, and genesis
///   configs
/// * `'b` - Lifetime parameter tied to the input block reference
/// * `'x` - Lifetime parameter tied to the auxiliary input data
pub fn asm_stf<'b, 'x, S: AsmSpec>(
    spec: &S,
    pre_state: &AnchorState,
    input: AsmStfInput<'b, 'x>,
) -> AsmResult<AsmStfOutput> {
    // 1. Validate and update PoW header continuity for the new block.
    // This ensures the block header follows proper Bitcoin consensus rules and chain continuity.
    let mut pow_state = pre_state.chain_view.pow_state.clone();
    pow_state
        .check_and_update_continuity(input.header, &Params::MAINNET)
        .map_err(AsmError::InvalidL1Header)?;

    let mut manager = SubprotoManager::new(spec, pre_state, input.aux_input);

    // 3. PROCESS: Feed each subprotocol its filtered transactions for execution.
    manager.invoke_process_txs(&input.protocol_txs, pre_state);

    // 4. FINISH: Allow each subprotocol to process buffered inter-protocol messages.
    // This stage handles cross-protocol communication and finalizes state changes.
    manager.invoke_process_msgs();

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
