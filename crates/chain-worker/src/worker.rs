//! Chain executor worker task.
//!
//! Responsible for managing the chainstate database as we receive orders to
//! apply/rollback blocks, DA, etc.

use std::sync::Arc;

use strata_chainexec::{
    BlockExecutionOutput, ChainExecutor, ExecContext, ExecResult, MemStateAccessor,
};
use strata_chaintsn::context::L2HeaderAndParent;
use strata_checkpoint_types::EpochSummary;
use strata_eectl::handle::ExecCtlHandle;
use strata_ol_chainstate_types::Chainstate;
use strata_primitives::{params::Params, prelude::*};
use strata_state::prelude::*;
use strata_status::StatusChannel;
use strata_tasks::ShutdownGuard;
use tokio::{
    runtime::Handle,
    sync::{Mutex, mpsc},
};
use tracing::*;

use crate::{
    WorkerContext, WorkerError, WorkerResult,
    handle::{ChainWorkerInput, WorkerShared},
    message::ChainWorkerMessage,
};

/// `StateAccessor` impl we pass to chaintsn.  Aliased here for convenience.
#[allow(dead_code)]
type AccessorImpl = MemStateAccessor;

/// Internal worker task state.
///
/// Has utility functions for basic tasks.
#[allow(dead_code)]
#[derive(Debug)]
pub struct WorkerState<W: WorkerContext> {
    /// Shared state between the worker and the handle.
    shared: Arc<Mutex<WorkerShared>>,

    /// Context for us to interface with the underlying system.
    context: W,

    /// Chain executor we call out to actually update the underlying state.
    chain_exec: ChainExecutor,

    exec_ctl_handle: ExecCtlHandle,
    // Current chain tip.
    // TODO remove this, not needed
    cur_tip: L2BlockCommitment,
    // Previous epoch that we're building upon.
    // prev_epoch: Option<EpochCommitment>,
}

#[allow(dead_code)]
impl<W: WorkerContext> WorkerState<W> {
    fn new(
        shared: Arc<Mutex<WorkerShared>>,
        context: W,
        chain_exec: ChainExecutor,
        exec_ctl_handle: ExecCtlHandle,
        cur_tip: L2BlockCommitment,
        // prev_epoch: Option<EpochCommitment>,
    ) -> Self {
        Self {
            shared,
            context,
            chain_exec,
            cur_tip,
            exec_ctl_handle, /* cur_tip,
                              * prev_epoch, */
        }
    }

    // /// Gets the current epoch we're in.
    // fn cur_epoch(&self) -> u64 {
    //     self.prev_epoch.epoch() + 1
    // }

    /// Prepares context for a block we're about to execute.
    fn prepare_block_context<'w>(
        &'w self,
        _l2bc: &L2BlockCommitment,
    ) -> WorkerResult<WorkerExecCtxImpl<'w, W>> {
        Ok(WorkerExecCtxImpl {
            worker_context: &self.context,
        })
    }

    // /// Prepares a new state accessor for the current tip state.
    // fn prepare_cur_state_accessor(&self) -> WorkerResult<AccessorImpl> {
    //     let wb = self
    //         .context
    //         .fetch_block_write_batch(self.cur_tip.blkid())?
    //         .ok_or(WorkerError::MissingBlockOutput(self.cur_tip))?;

    //     Ok(MemStateAccessor::new(wb.into_toplevel()))
    // }

    /// Updates the current tip as managed by the worker.  This does not persist
    /// in the client's database necessarily.
    fn update_cur_tip(&mut self, tip: L2BlockCommitment) -> WorkerResult<()> {
        self.cur_tip = tip;

        // Try to execute the payload, seeing if *that's* valid.
        self.exec_ctl_handle
            .update_safe_tip_blocking(tip)
            .map_err(WorkerError::ExecEnvEngine)?;

        Ok(())
    }

    fn try_exec_block(&mut self, block: &L2BlockCommitment) -> WorkerResult<()> {
        let blkid = block.blkid();

        debug!(%blkid, "Trying to execute block");

        // Prepare execution dependencies.
        let bundle = self
            .context
            .fetch_block(block.blkid())?
            .ok_or(WorkerError::MissingL2Block(*block.blkid()))?;

        let is_epoch_terminal = !bundle.body().l1_segment().new_manifests().is_empty();

        let parent_blkid = bundle.header().header().parent();
        let parent_header = self
            .context
            .fetch_header(parent_blkid)?
            .ok_or(WorkerError::MissingL2Block(*parent_blkid))?;

        // Try to execute the payload, seeing if *that's* valid.
        self.exec_ctl_handle
            .try_exec_el_payload_blocking(*block)
            .map_err(|_| WorkerError::InvalidExecPayload(*block))?;

        let header_ctx = L2HeaderAndParent::new(
            bundle.header().header().clone(),
            *parent_blkid,
            parent_header,
        );

        let exec_ctx = self.prepare_block_context(block)?;

        // Invoke the executor and produce an output.
        let output = self
            .chain_exec
            .verify_block(&header_ctx, bundle.body(), &exec_ctx)?;

        // Also, do whatever we have to do to complete the epoch.
        if is_epoch_terminal {
            debug!(%is_epoch_terminal);
            self.handle_complete_epoch(block.blkid(), bundle.block(), &output)?;
        }

        // Finally, we can persist the outputs.
        self.context.store_block_output(block.blkid(), &output)?;

        Ok(())
    }

    /// Takes the block and post-state and inserts database entries to reflect
    /// the epoch being finished on-chain.
    ///
    /// There's some bookkeeping here that's slightly weird since in the way it
    /// works now, the last block of an epoch brings the post-state to the new
    /// epoch.  So the epoch's final state actually has cur_epoch be the *next*
    /// epoch.  And the index we assign to the summary here actually uses the
    /// "prev epoch", since that's what the epoch in question is here.
    ///
    /// This will be simplified if/when we out the per-block and per-epoch
    /// processing into two separate stages.
    fn handle_complete_epoch(
        &mut self,
        blkid: &L2BlockId,
        block: &L2Block,
        last_block_output: &BlockExecutionOutput,
    ) -> WorkerResult<()> {
        // Construct the various parts of the summary
        // NOTE: epoch update in chainstate happens at first slot of next epoch
        // this code runs at final slot of current epoch.
        let output_tl_chs = last_block_output.write_batch().new_toplevel_state();

        let prev_epoch_idx = output_tl_chs.cur_epoch();
        let prev_terminal = output_tl_chs.prev_epoch().to_block_commitment();

        let slot = block.header().slot();
        let terminal = L2BlockCommitment::new(slot, *blkid);

        let l1seg = block.l1_segment();
        assert!(
            !l1seg.new_manifests().is_empty(),
            "chainworker: epoch finished without L1 records"
        );
        let new_tip_height = l1seg.new_height();
        let new_tip_blkid = l1seg.new_tip_blkid().expect("fcm: missing l1seg final L1");
        let new_l1_block = L1BlockCommitment::new(new_tip_height, new_tip_blkid);

        let epoch_final_state = last_block_output.computed_state_root();

        // Actually construct and insert the epoch summary.
        let summary = EpochSummary::new(
            prev_epoch_idx,
            terminal,
            prev_terminal,
            new_l1_block,
            *epoch_final_state,
        );

        // TODO convert to Display
        debug!(?summary, "completed chain epoch");

        self.context.store_summary(summary)?;

        Ok(())
    }

    fn finalize_epoch(&mut self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.exec_ctl_handle
            .update_finalized_tip_blocking(epoch.to_block_commitment())
            .map_err(WorkerError::ExecEnvEngine)?;

        Ok(())
    }
}

pub fn init_worker_state<W: WorkerContext>(
    shared: Arc<Mutex<WorkerShared>>,
    context: W,
    chain_exec: ChainExecutor,
    exec_ctl_handle: ExecCtlHandle,
    cur_tip: L2BlockCommitment,
    // prev_epoch: Option<EpochCommitment>,
) -> anyhow::Result<WorkerState<W>> {
    Ok(WorkerState::new(
        shared,
        context,
        chain_exec,
        exec_ctl_handle,
        cur_tip,
    ))
}

fn worker_task_inner<W: WorkerContext>(
    shutdown: &ShutdownGuard,
    mut state: WorkerState<W>,
    mut input: ChainWorkerInput,
) -> anyhow::Result<()> {
    info!("Starting chainworker task");
    while let Some(m) = input.recv_next() {
        match m {
            ChainWorkerMessage::TryExecBlock(l2bc, completion) => {
                let res = state.try_exec_block(&l2bc);
                let _ = completion.send(res);
            }

            ChainWorkerMessage::UpdateSafeTip(l2bc, completion) => {
                let res = state.update_cur_tip(l2bc);
                let _ = completion.send(res);
            }

            ChainWorkerMessage::FinalizeEpoch(epoch, completion) => {
                let res = state.finalize_epoch(epoch);
                let _ = completion.send(res);
            }
        }

        if shutdown.should_shutdown() {
            warn!("chain worker task received shutdown signal");
            break;
        }
    }

    Ok(())
}

pub struct WorkerExecCtxImpl<'c, W> {
    pub worker_context: &'c W,
}

impl<'c, W: WorkerContext> ExecContext for WorkerExecCtxImpl<'c, W> {
    fn fetch_l2_header(&self, blkid: &L2BlockId) -> ExecResult<L2BlockHeader> {
        self.worker_context
            .fetch_header(blkid)?
            .ok_or(strata_chainexec::Error::MissingL2Header(*blkid))
    }

    fn fetch_block_toplevel_post_state(&self, blkid: &L2BlockId) -> ExecResult<Chainstate> {
        // This impl might be suboptimal, should we do real reconstruction?
        //
        // Maybe actually make this return a `StateAccessor` already?
        let wb = self
            .worker_context
            .fetch_block_write_batch(blkid)?
            .ok_or(strata_chainexec::Error::MissingBlockPostState(*blkid))?;
        Ok(wb.into_toplevel())
    }
}

/// Internal function to spawn a chain worker with encapsulated initialization.
///
/// This function contains all the initialization logic that was previously exposed
/// in the consensus-logic crate, including genesis waiting and tip resolution.
#[allow(clippy::too_many_arguments)]
pub(crate) fn worker_task<W: WorkerContext>(
    shutdown: ShutdownGuard,
    handle: Handle,
    context: W,
    status_channel: StatusChannel,
    params: Arc<Params>,
    exec_ctl_handle: ExecCtlHandle,
    chain_msg_rx: mpsc::Receiver<ChainWorkerMessage>,
    shared: Arc<Mutex<WorkerShared>>,
) -> anyhow::Result<()> {
    // Wait for genesis and determine the current tip
    let cur_tip = wait_for_genesis_and_resolve_tip(&status_channel, &context, &handle)?;

    let blkid = *cur_tip.blkid();
    info!(%blkid, "starting chain worker");

    // Create the chain executor
    let chain_exec = ChainExecutor::new(params.rollup().clone());

    // Initialize shared state and worker state
    let state = init_worker_state(
        shared.clone(),
        context,
        chain_exec,
        exec_ctl_handle,
        cur_tip,
    )?;

    // Create input receiver
    let input = ChainWorkerInput::new(shared, chain_msg_rx);

    // Run the worker task
    worker_task_inner(&shutdown, state, input)?;

    Ok(())
}

/// Wait for genesis and resolve the current tip block commitment.
///
/// This encapsulates the complex logic for determining the initial state
/// that the worker should start from.
fn wait_for_genesis_and_resolve_tip<W: WorkerContext>(
    status_channel: &StatusChannel,
    ctx: &W,
    handle: &Handle,
) -> WorkerResult<L2BlockCommitment> {
    info!("waiting until genesis");

    let init_state = handle
        .block_on(status_channel.wait_until_genesis())
        .map_err(|e| WorkerError::Unexpected(format!("failed to wait for genesis: {e}")))?;

    let genesis_blkid = handle.block_on(async {
        while ctx.fetch_block_ids(0).unwrap().is_empty() {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
        *ctx.fetch_block_ids(0).unwrap().first().unwrap()
    });

    let cur_tip = match init_state.get_declared_final_epoch() {
        Some(epoch) => epoch.to_block_commitment(),
        None => L2BlockCommitment::new(0, genesis_blkid),
    };

    Ok(cur_tip)
}
