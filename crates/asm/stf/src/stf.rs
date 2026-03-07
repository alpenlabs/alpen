//! The `asm_stf` crate implements the core Anchor State Machine state transition function (STF). It
//! glues together block‐level validation, a set of pluggable subprotocols, and the global chain
//! view into a single deterministic state transition.
// TODO rename this module to `transition`

use strata_asm_common::{
    AnchorState, AsmError, AsmManifest, AsmResult, AsmSpec, ChainViewState, VerifiedAuxData,
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
pub fn compute_asm_transition<'i, S: AsmSpec>(
    spec: &S,
    pre_state: &AnchorState,
    input: AsmStfInput<'i>,
) -> AsmResult<AsmStfOutput> {
    // 1. Validate and update PoW header continuity for the new block.
    // This ensures the block header follows proper Bitcoin consensus rules and chain continuity.
    let (mut pow_state, mut history_accumulator) = pre_state.chain_view.clone().into_parts();
    pow_state
        .check_and_update(input.header)
        .map_err(AsmError::InvalidL1Header)?;

    let verified_aux_data =
        VerifiedAuxData::try_new(&input.aux_data, &pre_state.chain_view.history_accumulator)?;

    // After `check_and_update`, `last_verified_block` points to the block we
    // just validated — i.e. the L1 block whose transactions we are about to
    // feed into subprotocols.
    let current_l1ref = &pow_state.last_verified_block;

    let mut manager = SubprotoManager::new();

    // 2. LOAD: Initialize each subprotocol in the subproto manager with aux input data.
    let mut loader = AnchorStateLoader::new(pre_state, &mut manager);
    spec.load_subprotocols(&mut loader);

    // 3. PROCESS: Feed each subprotocol its filtered transactions for execution.
    // This stage performs the actual state transitions for each subprotocol.
    let mut process_stage = ProcessStage::new(
        &mut manager,
        current_l1ref,
        input.protocol_txs,
        verified_aux_data,
    );
    spec.call_subprotocols(&mut process_stage);

    // 4. FINISH: Allow each subprotocol to process buffered inter-protocol messages.
    // This stage handles cross-protocol communication and finalizes state changes.
    // TODO probably will have change this to repeat the interproto message
    // processing phase until we have no more messages to deliver, or some
    // bounded number of times
    let mut finish_stage = FinishStage::new(&mut manager, &pow_state.last_verified_block);
    spec.call_subprotocols(&mut finish_stage);

    // 5. Construct the manifest with the logs.
    let (sections, logs) = manager.export_sections_and_logs();
    let manifest = AsmManifest::new(
        current_l1ref.height(),
        *current_l1ref.blkid(),
        input.wtxids_root.into(),
        logs,
    );

    // 6. Append the manifest to the history accumulator
    history_accumulator.add_manifest(&manifest)?;

    // 7. Construct the final `AnchorState` and output.
    let chain_view = ChainViewState {
        pow_state,
        history_accumulator,
    };
    let state = AnchorState {
        chain_view,
        sections,
    };
    let output = AsmStfOutput { state, manifest };
    Ok(output)
}
