//! CSM worker service implementation.

use std::{marker::PhantomData, sync::Arc};

use strata_asm_worker::AsmWorkerStatus;
use strata_csm_types::{ClientState, ClientUpdateOutput, L1Checkpoint};
use strata_primitives::l1::L1BlockCommitment;
use strata_service::{Response, Service, SyncService};
use tracing::*;

use crate::{context::CsmWorkerContext, state::CsmWorkerState, status::CsmWorkerStatus};

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

        // Process `asm_block` and any blocks that might have been skipped.
        //
        // Errors here are intentionally swallowed: gap-fill is idempotent and
        // the next ASM status update will retry the missed blocks.
        if let Err(e) = state.process_asm_block(asm_block, asm_status.logs()) {
            error!(%asm_block, err = ?e, "Failed to process ASM block");
        }

        let newly_finalized = state.advance_finalization(asm_block.height());
        let confirmed_changed = state.confirmed_epoch != prev_confirmed_epoch;

        // If depth-driven finality advanced, refresh ClientState's
        // `last_finalized_checkpoint` so persisted state matches the worker's
        // depth-derived view. ClientState is the only thing downstream consumers
        // (fork choice, chain worker, RPC) can see, so it has to carry the truth.
        if let Some(finalized) = newly_finalized.as_ref()
            && let Err(e) = refresh_finalized_checkpoint(state, asm_block, finalized.clone())
        {
            error!(%asm_block, err = ?e, "Failed to persist refreshed finalized checkpoint");
        }

        // FCM listens on checkpoint-state updates. Emit when checkpoint status changed
        // even if client-state object itself did not change (tip-only L1 movement).
        if confirmed_changed || newly_finalized.is_some() {
            state
                .ctx
                .publish_client_state(state.last_committed_state.as_ref().clone(), asm_block);
        }

        Ok(Response::Continue)
    }
}

/// Rewrites the committed `ClientState` so its `last_finalized_checkpoint`
/// matches the depth-finalized `L1Checkpoint`, then re-persists it under
/// `asm_block`. The publish happens in the caller alongside other status edges.
fn refresh_finalized_checkpoint<C: CsmWorkerContext>(
    state: &mut CsmWorkerState<C>,
    asm_block: L1BlockCommitment,
    finalized: L1Checkpoint,
) -> anyhow::Result<()> {
    let last_seen = state.last_committed_state.get_last_checkpoint();
    let refreshed = ClientState::new(Some(finalized), last_seen);
    state.ctx.put_client_state_update(
        &asm_block,
        ClientUpdateOutput::new(refreshed.clone(), vec![]),
    )?;
    state.last_committed_state = Arc::new(refreshed);
    Ok(())
}
