//! The `asm_stf` crate implements the core Anchor State Machine state transition function (STF). It
//! glues together block‐level validation, a set of pluggable subprotocols, and the global chain
//! view into a single deterministic state transition.
// TODO rename this module to `transition`

use strata_asm_common::{
    AnchorState, AsmError, AsmManifest, AsmMmr, AsmResult, AsmSpec, ChainViewState,
};

use crate::{
    manager::{AnchorStateLoader, SubprotoManager},
    stage::{FinishStage, ProcessStage},
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
pub fn compute_asm_transition<'i, S: AsmSpec>(
    spec: &S,
    pre_state: &AnchorState,
    input: AsmStfInput<'i>,
) -> AsmResult<AsmStfOutput> {
    // 1. Validate and update PoW header continuity for the new block.
    // This ensures the block header follows proper Bitcoin consensus rules and chain continuity.
    let (mut pow_state, mmr) = pre_state.chain_view.clone().into_parts();
    pow_state
        .check_and_update(input.header)
        .map_err(AsmError::InvalidL1Header)?;

    let mut manager = SubprotoManager::new();

    // 2. LOAD: Initialize each subprotocol in the subproto manager with aux input data.
    let mut loader = AnchorStateLoader::new(pre_state, &mut manager);
    spec.load_subprotocols(&mut loader);

    // 3. PROCESS: Feed each subprotocol its filtered transactions for execution.
    // This stage performs the actual state transitions for each subprotocol.
    let mut process_stage =
        ProcessStage::new(&mut manager, pre_state, input.protocol_txs, &input.aux_data);
    spec.call_subprotocols(&mut process_stage);

    // 4. FINISH: Allow each subprotocol to process buffered inter-protocol messages.
    // This stage handles cross-protocol communication and finalizes state changes.
    // TODO probably will have change this to repeat the interproto message
    // processing phase until we have no more messages to deliver, or some
    // bounded number of times
    let mut finish_stage = FinishStage::new(&mut manager);
    spec.call_subprotocols(&mut finish_stage);

    // 5. Construct the manifest with the logs.
    let (sections, logs) = manager.export_sections_and_logs();
    let manifest = AsmManifest::new(
        *pow_state.last_verified_block.blkid(),
        input.wtxids_root,
        logs,
    );

    // 6. Append the manifest root to the MMR
    let mut mmr: AsmMmr = mmr.into();
    mmr.add_leaf(manifest.compute_hash())?;
    let manifest_mmr = mmr.into();

    // 7. Construct the final `AnchorState` and output.
    let chain_view = ChainViewState {
        pow_state,
        manifest_mmr,
    };
    let state = AnchorState {
        chain_view,
        sections,
    };
    let output = AsmStfOutput { state, manifest };
    Ok(output)
}
