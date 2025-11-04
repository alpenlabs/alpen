//! The `asm_stf` crate implements the core Anchor State Machine state transition function (STF). It
//! glues together block‐level validation, a set of pluggable subprotocols, and the global chain
//! view into a single deterministic state transition.
// TODO rename this module to `transition`

use bitcoin::hashes::Hash;
use strata_asm_chain_types::{AsmLog, AsmManifest};
use strata_asm_common::{AnchorState, AsmError, AsmResult, AsmSpec, ChainViewState, Mmr64};
use strata_identifiers::Buf32;

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
    let AsmStfInput {
        header,
        protocol_txs,
        aux_responses,
        wtx_root,
    } = input;

    // 1. Validate and update PoW header continuity for the new block.
    // This ensures the block header follows proper Bitcoin consensus rules and chain continuity.
    let mut pow_state = pre_state.chain_view.pow_state.clone();
    pow_state
        .check_and_update(header)
        .map_err(AsmError::InvalidL1Header)?;

    let mut manager = SubprotoManager::new();

    // 2. LOAD: Initialize each subprotocol in the subproto manager with aux input data.
    // todo: find proper place for aux data verification maybe in loader or here before process
    // stage
    let mut loader = AnchorStateLoader::new(pre_state, &mut manager);
    spec.load_subprotocols(&mut loader);

    // 3. PROCESS: Feed each subprotocol its filtered transactions for execution.
    // This stage performs the actual state transitions for each subprotocol.
    let mut process_stage = ProcessStage::new(&mut manager, pre_state, protocol_txs, aux_responses);
    spec.call_subprotocols(&mut process_stage);

    // 4. FINISH: Allow each subprotocol to process buffered inter-protocol messages.
    // This stage handles cross-protocol communication and finalizes state changes.
    // TODO probably will have change this to repeat the interproto message
    // processing phase until we have no more messages to deliver, or some
    // bounded number of times
    let mut finish_stage = FinishStage::new(&mut manager);
    spec.call_subprotocols(&mut finish_stage);

    // 5. Construct the final `AnchorState` and output.
    // Export the updated state sections and logs from all subprotocols to build the result.
    let (sections, logs) = manager.export_sections_and_logs();

    // Convert AsmLogEntry to AsmLog for the manifest
    let manifest_logs: Vec<AsmLog> = logs
        .iter()
        .map(|entry| AsmLog::from(entry.as_bytes().to_vec()))
        .collect();

    // Compute the block hash from the header
    let block_hash = header.block_hash();
    let block_root = Buf32::from(*block_hash.as_byte_array());

    let manifest = AsmManifest::new(block_root, wtx_root, manifest_logs);
    let manifest_hash: [u8; 32] = manifest.compute_hash();

    let mut history_mmr = Mmr64::from_compact(&pre_state.chain_view.history_mmr);
    history_mmr
        .add_leaf(manifest_hash)
        .map_err(AsmError::HeaderMmr)?;

    let chain_view = ChainViewState {
        pow_state,
        history_mmr: history_mmr.to_compact(),
    };
    let state = AnchorState {
        chain_view,
        sections,
    };
    let output = AsmStfOutput { state, logs };
    Ok(output)
}
