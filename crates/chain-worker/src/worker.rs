//! Chain executor worker task.
//!
//! Responsible for managing the chainstate database as we receive orders to
//! apply/rollback blocks, DA, etc.

use std::sync::Arc;

use strata_chainexec::{
    BlockExecutionOutput, ChainExecutor, ExecContext, ExecResult, MemStateAccessor,
};
use strata_chaintsn::context::L2HeaderAndParent;
use strata_common::retry::{
    DEFAULT_ENGINE_CALL_MAX_RETRIES, policies::ExponentialBackoff, retry_with_backoff,
};
use strata_eectl::{engine::ExecEngineCtl, errors::EngineError, messages::ExecPayloadData};
use strata_primitives::{batch::EpochSummary, prelude::*};
use strata_state::{block::L2BlockBundle, chain_state::Chainstate, header::L2Header, prelude::*};
use tracing::*;

use crate::{
    WorkerContext, WorkerError, WorkerResult,
    handle::{ChainWorkerInput, WorkerShared},
    message::WorkerMessage,
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
        self.call_engine("engine_update_safe", |eng| {
            eng.update_safe_block(*tip.blkid())
        })?;
        Ok(())
    }

    fn try_exec_block(&mut self, block: &L2BlockCommitment) -> WorkerResult<()> {
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
        self.try_exec_el_payload(block.blkid(), &bundle)?;

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

        // Also, do whatever we have to do to complete the epoch.
        if is_epoch_terminal {
            self.handle_complete_epoch(block.blkid(), bundle.block(), &output)?;
        }

        self.context.store_block_output(block.blkid(), output)?;

        // Update the tip we've processed.
        self.update_cur_tip(*block)?;

        Ok(())
    }

    fn try_exec_el_payload(
        &mut self,
        blkid: &L2BlockId,
        bundle: &L2BlockBundle,
    ) -> WorkerResult<()> {
        // We don't do this for the genesis block because that block doesn't
        // actually have a well-formed accessory and it gets mad at us.
        if bundle.header().slot() == 0 {
            return Ok(());
        }

        // Construct the exec payload and just make the call.  This blocks until
        // it gets back to us, which kinda sucks, but we're working on it!
        let exec_hash = bundle.header().exec_payload_hash();
        let eng_payload = ExecPayloadData::from_l2_block_bundle(bundle);
        let res = self.call_engine("engine_submit_payload", move |eng| {
            // annoying that we're cloning this each time, maybe make it take a ref?
            eng.submit_payload(eng_payload.clone())
        })?;

        if res == strata_eectl::engine::BlockStatus::Invalid {
            let block = L2BlockCommitment::new(bundle.header().slot(), *blkid);
            Err(WorkerError::InvalidExecPayload(block).into())
        } else {
            Ok(())
        }
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
        let output_tl_chs = last_block_output.changes().toplevel();

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
        // TODO apply outputs that haven't been merged, etc.

        self.call_engine("engine_update_finalized", |eng| {
            eng.update_finalized_block(*epoch.last_blkid())
        })?;

        Err(WorkerError::Unimplemented)
    }

    /// Make a call to the exec engine, using retry and backoff.
    fn call_engine<T>(
        &mut self,
        name: &str,
        f: impl Fn(&mut E) -> Result<T, EngineError>,
    ) -> WorkerResult<T> {
        let res = retry_with_backoff(
            name,
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            move || f(&mut self.engine),
        )?;
        Ok(res)
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

            WorkerMessage::FinalizeEpoch(epoch, completion) => {
                let res = state.finalize_epoch(epoch);
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
