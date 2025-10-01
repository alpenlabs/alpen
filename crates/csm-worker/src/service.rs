//! CSM worker service implementation.

use strata_asm_worker::AsmWorkerStatus;
use strata_service::{Response, Service, SyncService};
use tracing::*;

use crate::{processor::process_checkpoint_log, state::CsmWorkerState, status::CsmWorkerStatus};

/// CSM worker service that acts as a listener to ASM worker status updates.
///
/// This service monitors ASM worker and reacts to checkpoint logs emitted by the
/// checkpoint-v0 subprotocol. When ASM processes a checkpoint transaction, it emits
/// a `CheckpointUpdate` log which this service processes to update the client state.
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
            last_processed_epoch: state.last_processed_epoch,
        }
    }
}

impl SyncService for CsmWorkerService {
    fn process_input(state: &mut Self::State, asm_status: &Self::Msg) -> anyhow::Result<Response> {
        // Extract the current block from ASM status
        let Some(asm_block) = asm_status.cur_block else {
            // ASM hasn't processed any blocks yet
            trace!("ASM status has no current block, skipping");
            return Ok(Response::Continue);
        };

        info!(%asm_block, "CSM is processing ASM logs.");

        // Track which block we're processing
        state.last_asm_block = Some(asm_block);

        // Extract checkpoint logs from ASM status
        let logs = asm_status.logs();

        if logs.is_empty() {
            trace!(%asm_block, "No logs in ASM status update");
            return Ok(Response::Continue);
        }

        // Process each checkpoint update log
        for log in logs {
            if let Err(e) = process_checkpoint_log(state, log, &asm_block) {
                error!(%asm_block, err = %e, "Failed to process checkpoint log");
                // Continue processing other logs instead of failing completely
            }
        }

        info!(%asm_block, "CSM successfully processed ASM logs.");

        Ok(Response::Continue)
    }
}
