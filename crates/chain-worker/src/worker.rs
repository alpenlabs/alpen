//! Chain executor worker task.
//!
//! Responsible for managing the chainstate database as we receive orders to
//! apply/rollback blocks, DA, etc.

use std::sync::Arc;

use strata_chainexec::{ChainExecutor, ExecContext, ExecResult, MemStateAccessor};
use strata_chaintsn::context::L2HeaderAndParent;
use strata_eectl::engine::ExecEngineCtl;
use strata_primitives::prelude::*;
use strata_state::{chain_state::Chainstate, header::L2Header, prelude::*};

use crate::{
    WorkerContext, WorkerError, WorkerResult,
    handle::{ChainWorkerInput, WorkerMessage, WorkerShared},
};

/// `StateAccessor` impl we pass to chaintsn.  Aliased here for convenience.
type AccessorImpl = MemStateAccessor;

/// Internal worker task state.
///
/// Has utility functions for basic tasks.
pub struct WorkerState<W: WorkerContext, E> {
    /// Shared state between the worker and the handle.
    shared: Arc<WorkerShared>,

    /// Context for us to interface with the underlying system.
    context: W,

    /// Chain executor we call out to actually update the underlying state.
    chain_exec: ChainExecutor,

    /// Execution engine controller.
    ///
    /// This will eventually be refactored out.
    engine: E,

    /// Current chain tip.
    cur_tip: L2BlockCommitment,

    /// Previous epoch that we're building upon.
    prev_epoch: EpochCommitment,
}

impl<W: WorkerContext, E: ExecEngineCtl> WorkerState<W, E> {
    /// Gets the current epoch we're in.
    fn cur_epoch(&self) -> u64 {
        self.prev_epoch.epoch() + 1
    }

    /// Prepares context for a block we're about to execute.
    fn prepare_block_context<'w>(
        &'w self,
        l2bc: &L2BlockCommitment,
    ) -> WorkerResult<WorkerExecCtxImpl<'w, W>> {
        Ok(WorkerExecCtxImpl {
            worker_context: &self.context,
        })
    }

    /// Prepares a new state accessor for the current tip state.
    fn prepare_cur_state_accessor(&self) -> WorkerResult<AccessorImpl> {
        let output = self
            .context
            .fetch_block_output(self.cur_tip.blkid())?
            .ok_or(WorkerError::MissingBlockOutput(self.cur_tip))?;

        Ok(MemStateAccessor::new(output.changes().toplevel().clone()))
    }

    /// Updates the current tip as managed by the worker.  This does not persist
    /// in the client's database necessarily.
    fn update_cur_tip(&mut self, tip: L2BlockCommitment) -> WorkerResult<()> {
        self.cur_tip = tip;
        self.engine.update_safe_block(*tip.blkid())?;
        Ok(())
    }

    fn try_exec_block(&mut self, block: &L2BlockCommitment) -> WorkerResult<()> {
        // Prepare execution dependencies.
        let bundle = self
            .context
            .fetch_block(block.blkid())?
            .ok_or(WorkerError::MissingL2Block(*block.blkid()))?;

        let parent_blkid = bundle.header().header().parent();
        let parent_header = self
            .context
            .fetch_header(parent_blkid)?
            .ok_or(WorkerError::MissingL2Block(*parent_blkid))?;

        let header_ctx = L2HeaderAndParent::new(
            bundle.header().header().clone(),
            *parent_blkid,
            parent_header,
        );

        let exec_ctx = self.prepare_block_context(block)?;

        // Invoke the executor and produce an output.
        let output = self
            .chain_exec
            .execute_block(&header_ctx, bundle.body(), &exec_ctx)?;
        self.context.store_block_output(block.blkid(), output)?;

        // Update the tip we've processed.
        self.update_cur_tip(*block)?;

        Ok(())
    }
}

pub fn worker_task<W: WorkerContext, E: ExecEngineCtl>(
    mut state: WorkerState<W, E>,
    mut input: ChainWorkerInput,
) -> anyhow::Result<()> {
    while let Some(m) = input.recv_next() {
        match m {
            WorkerMessage::TryExecBlock(l2bc, completion) => {
                let res = state.try_exec_block(&l2bc);
                let _ = completion.send(res);
            }
        }
    }

    Ok(())
}

struct WorkerExecCtxImpl<'c, W> {
    worker_context: &'c W,
}

impl<'c, W: WorkerContext> ExecContext for WorkerExecCtxImpl<'c, W> {
    fn fetch_l2_header(&self, blkid: &L2BlockId) -> ExecResult<L2BlockHeader> {
        self.worker_context
            .fetch_header(blkid)
            .map_err(|e| <WorkerError as Into<strata_chainexec::Error>>::into(e))?
            .ok_or(strata_chainexec::Error::MissingL2Header(*blkid))
    }

    fn fetch_block_toplevel_post_state(&self, blkid: &L2BlockId) -> ExecResult<Chainstate> {
        // This impl is suboptimal, we should do some real reconstruction.
        //
        // Maybe actually make this return a `StateAccessor` already?
        let output = self
            .worker_context
            .fetch_block_output(blkid)
            .map_err(|e| <WorkerError as Into<strata_chainexec::Error>>::into(e))?
            .ok_or(strata_chainexec::Error::MissingBlockPostState(*blkid))?;
        Ok(output.changes().toplevel().clone())
    }
}
