//! CSM worker service implementation.

use strata_asm_worker::AsmWorkerStatus;
use strata_service::{Response, Service, SyncService};
use tracing::*;

use crate::{processor::process_log, state::CsmWorkerState, status::CsmWorkerStatus};

/// CSM worker service that acts as a listener to ASM worker status updates.
///
/// This service monitors ASM worker and reacts to checkpoint logs emitted by the
/// checkpoint subprotocols. When ASM processes a checkpoint transaction, it emits
/// a checkpoint log (`CheckpointTipUpdate` in v1, legacy `CheckpointUpdate` in v0)
/// which this service processes to update the client state.
///
/// The service follows the listener pattern - it passively observes ASM status updates
/// via the service framework's `StatusMonitorInput` without ASM being aware of it.
#[derive(Debug)]
pub struct CsmWorkerService;

impl Service for CsmWorkerService {
    type State = CsmWorkerState;
    type Msg = AsmWorkerStatus;
    type Status = CsmWorkerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        CsmWorkerStatus {
            cur_block: state.last_asm_block,
            last_processed_epoch: state.last_processed_epoch.map(|e| e as u64),
            last_confirmed_epoch: state.confirmed_epoch,
            last_finalized_epoch: state.finalized_epoch,
        }
    }
}

impl SyncService for CsmWorkerService {
    fn process_input(state: &mut Self::State, asm_status: Self::Msg) -> anyhow::Result<Response> {
        strata_common::check_bail_trigger(strata_common::BAIL_CSM_EVENT);

        // Extract the current block from ASM status
        let Some(asm_block) = asm_status.cur_block else {
            // ASM hasn't processed any blocks yet
            trace!("ASM status has no current block, skipping");
            return Ok(Response::Continue);
        };

        trace!("CSM is processing ASM logs.");

        // Track which block we're processing
        state.last_asm_block = Some(asm_block);
        let prev_confirmed_epoch = state.confirmed_epoch;
        let prev_finalized_epoch = state.finalized_epoch;

        // Process checkpoint logs from ASM status
        for log in asm_status.logs() {
            if let Err(e) = process_log(state, log, &asm_block) {
                error!(%asm_block, err = %e, "Failed to process ASM log");
            }
        }

        // Advance finalized epoch from the observation queue based on L1 depth.
        let current_l1_tip = asm_block.height();
        let finality_depth = state.params.rollup.l1_reorg_safe_depth.max(1);
        while let Some((commitment, observation)) = state.observed_checkpoints.front() {
            if state
                .finalized_epoch
                .is_some_and(|current| commitment.epoch() <= current.epoch())
            {
                state.observed_checkpoints.pop_front();
                continue;
            }

            let confirmations = current_l1_tip.saturating_sub(observation.l1_commitment.height());
            if confirmations >= finality_depth {
                let epoch = *commitment;
                state.observed_checkpoints.pop_front();
                if state
                    .finalized_epoch
                    .is_none_or(|current| epoch.epoch() > current.epoch())
                {
                    state.finalized_epoch = Some(epoch);
                }
            } else {
                break;
            }
        }

        // FCM listens on checkpoint-state updates. Emit when checkpoint status changed
        // even if client-state object itself did not change (tip-only L1 movement).
        if state.confirmed_epoch != prev_confirmed_epoch
            || state.finalized_epoch != prev_finalized_epoch
        {
            state
                .status_channel
                .update_client_state(state.cur_state.as_ref().clone(), asm_block);
        }

        Ok(Response::Continue)
    }
}
