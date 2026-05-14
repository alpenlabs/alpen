//! CSM worker service implementation.

use std::marker::PhantomData;

use strata_asm_worker::AsmWorkerStatus;
use strata_service::{Response, Service, SyncService};
use tracing::*;

use crate::{
    context::CsmWorkerContext,
    processor::{commit_block, process_log},
    state::CsmWorkerState,
    status::CsmWorkerStatus,
};

/// CSM worker service that acts as a listener to ASM worker status updates.
///
/// This service monitors ASM worker and reacts to checkpoint logs emitted by the
/// checkpoint subprotocol. When ASM processes a checkpoint transaction, it emits
/// a `CheckpointTipUpdate` log which this service processes to update the client state.
///
/// The service follows the listener pattern - it passively observes ASM status updates
/// via the service framework's `StatusMonitorInput` without ASM being aware of it.
#[derive(Debug)]
pub struct CsmWorkerService<C> {
    _ctx: PhantomData<C>,
}

impl<C: CsmWorkerContext + 'static> Service for CsmWorkerService<C> {
    type State = CsmWorkerState<C>;
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

impl<C: CsmWorkerContext + 'static> SyncService for CsmWorkerService<C> {
    fn process_input(state: &mut Self::State, asm_status: Self::Msg) -> anyhow::Result<Response> {
        strata_common::check_bail_trigger(strata_common::BAIL_CSM_EVENT);

        // Extract the current block from ASM status
        let Some(asm_block) = asm_status.cur_block else {
            // ASM hasn't processed any blocks yet
            trace!("ASM status has no current block, skipping");
            return Ok(Response::Continue);
        };

        trace!("CSM is processing ASM logs.");

        let prev_confirmed_epoch = state.confirmed_epoch;
        let prev_finalized_epoch = state.finalized_epoch;

        // Snapshot the in-memory client state so a mid-block failure can roll
        // back any partial update and leave the block to be re-processed.
        let cur_state_snapshot = state.cur_state.clone();

        // Process checkpoint logs from ASM status. Persist updates only if this succeeds.
        let mut block_ok = true;
        for log in asm_status.logs() {
            if let Err(e) = process_log(state, log, &asm_block) {
                error!(%asm_block, err = %e, "Failed to process ASM log");
                block_ok = false;
                break;
            }
        }

        if block_ok {
            if let Err(e) = commit_block(state, asm_block) {
                error!(%asm_block, err = %e, "Failed to commit CSM block");
                state.cur_state = cur_state_snapshot;
            }
        } else {
            // Roll back the partially-applied in-memory state.
            state.cur_state = cur_state_snapshot;
        }

        // Advance finalized epoch from the observation queue based on L1 depth.
        let current_l1_tip = asm_block.height();
        let finality_depth = state.ctx.l1_reorg_safe_depth().max(1);
        while let Some((commitment, observation)) = state.observed_checkpoints.front() {
            if state
                .finalized_epoch
                .is_some_and(|current| commitment.epoch() <= current.epoch())
            {
                state.observed_checkpoints.pop_front();
                continue;
            }

            let confirmations = current_l1_tip
                .saturating_sub(observation.l1_commitment.height())
                .saturating_add(1);
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
                .ctx
                .publish_client_state(state.cur_state.as_ref().clone(), asm_block);
        }

        Ok(Response::Continue)
    }
}
