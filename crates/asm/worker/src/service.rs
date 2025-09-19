//! Service framework integration for ASM.

use serde::Serialize;
use strata_primitives::prelude::*;
use strata_service::{Response, Service, SyncService};
use strata_state::asm_state::AsmState;
use tracing::*;

use crate::{AsmWorkerServiceState, traits::WorkerContext};

/// ASM service implementation using the service framework.
#[derive(Debug)]
pub struct AsmWorkerService<W> {
    _phantom: std::marker::PhantomData<W>,
}

impl<W: WorkerContext + Send + Sync + 'static> Service for AsmWorkerService<W> {
    type State = AsmWorkerServiceState<W>;
    type Msg = L1BlockCommitment;
    type Status = AsmWorkerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        AsmWorkerStatus {
            is_initialized: state.initialized,
            cur_block: state.blkid,
        }
    }
}

impl<W: WorkerContext + Send + Sync + 'static> SyncService for AsmWorkerService<W> {
    fn on_launch(state: &mut AsmWorkerServiceState<W>) -> anyhow::Result<()> {
        Ok(state.load_latest_or_create_genesis()?)
    }

    // TODO(QQ): add tests.
    fn process_input(
        state: &mut AsmWorkerServiceState<W>,
        incoming_block: &L1BlockCommitment,
    ) -> anyhow::Result<Response> {
        let ctx = &state.context;

        // Handle pre-genesis: if the block is before genesis we don't care about it.
        let genesis_height = state.params.rollup().genesis_l1_view.height();
        let height = incoming_block.height();
        if height < genesis_height {
            warn!(%height, "ignoring unexpected L1 block before genesis");
            return Ok(Response::Continue);
        }

        // Traverse back the chain of l1 blocks until we find an l1 block which has AnchorState.
        // Remember all the blocks along the way and pass it (in the reverse order) to process.
        let mut skipped_blocks = vec![];
        let mut pivot_block = *incoming_block;
        let mut pivot_anchor = ctx.get_anchor_state(&pivot_block);

        while pivot_anchor.is_err() && pivot_block.height() >= genesis_height {
            let block = ctx.get_l1_block(pivot_block.blkid())?;
            let parent_block_id = L1BlockCommitment::new(
                pivot_block.height() - 1,
                block.header.prev_blockhash.into(),
            );

            // Push the unprocessed block.
            skipped_blocks.push((block, pivot_block));

            // Update the loop state.
            pivot_anchor = ctx.get_anchor_state(&parent_block_id);
            pivot_block = parent_block_id;
        }

        // We reached the height before genesis (while traversing), but didn't find genesis state.
        if pivot_block.height() < genesis_height {
            warn!("ASM hasn't found pivot anchor state at genesis.");
            return Ok(Response::ShouldExit);
        }

        // Found pivot anchor state - our starting point.
        info!(%pivot_block, "ASM found pivot anchor state");
        state.update_anchor_state(pivot_anchor.unwrap(), pivot_block);

        // Process the whole chain of unprocessed blocks, starting from older blocks till
        // incoming_block.
        for (block, block_id) in skipped_blocks.iter().rev() {
            info!(%block_id, "ASM transition attempt");
            match state.transition(block) {
                Ok(asm_stf_out) => {
                    let new_state = AsmState::from_output(asm_stf_out);
                    // Store and update anchor.
                    state.context.store_anchor_state(block_id, &new_state)?;
                    state.update_anchor_state(new_state, *block_id);

                    state.notify_subprotocols()?;
                }
                Err(e) => {
                    error!(%e, "ASM transition error");
                    return Ok(Response::ShouldExit);
                }
            }
            info!(%block_id, "ASM transition success");
        }

        Ok(Response::Continue)
    }
}

/// Status information for the ASM worker service.
#[derive(Clone, Debug, Serialize)]
pub struct AsmWorkerStatus {
    pub is_initialized: bool,
    pub cur_block: Option<L1BlockCommitment>,
}
