//! Service framework integration for chain worker.

use std::sync::Arc;

use serde::Serialize;
use strata_chainexec::{BlockExecutionOutput, ChainExecutor};
use strata_eectl::handle::ExecCtlHandle;
use strata_primitives::{batch::EpochSummary, params::Params, prelude::*};
use strata_service::{Response, Service, ServiceState, SyncService};
use strata_state::{block::L2Block, header::L2Header};
use strata_status::StatusChannel;
use tokio::{runtime::Handle, sync::Mutex};
use tracing::*;

use crate::{
    context::WorkerExecCtxImpl,
    errors::{WorkerError, WorkerResult},
    handle::WorkerShared,
    message::ChainWorkerMessage,
    traits::WorkerContext,
};

/// Chain worker service implementation using the service framework.
#[derive(Debug)]
pub struct ChainWorkerService<W> {
    _phantom: std::marker::PhantomData<W>,
}

impl<W: WorkerContext + Send + Sync + 'static> Service for ChainWorkerService<W> {
    type State = ChainWorkerServiceState<W>;
    type Msg = ChainWorkerMessage;
    type Status = ChainWorkerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        ChainWorkerStatus {
            is_initialized: state.is_initialized(),
            cur_tip: state.cur_tip,
            last_finalized_epoch: state.last_finalized_epoch,
        }
    }
}

impl<W: WorkerContext + Send + Sync + 'static> SyncService for ChainWorkerService<W> {
    fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        let cur_tip = state.wait_for_genesis_and_resolve_tip()?;
        state.initialize_with_tip(cur_tip)?;
        Ok(())
    }

    fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            ChainWorkerMessage::TryExecBlock(l2bc, completion) => {
                let res = state.try_exec_block(l2bc);
                completion.send_blocking(res);
            }

            ChainWorkerMessage::UpdateSafeTip(l2bc, completion) => {
                let res = state.update_cur_tip(*l2bc);
                completion.send_blocking(res);
            }

            ChainWorkerMessage::FinalizeEpoch(epoch, completion) => {
                let res = state.finalize_epoch(*epoch);
                completion.send_blocking(res);
            }
        }

        Ok(Response::Continue)
    }
}

/// Service state for the chain worker.
#[derive(Debug)]
pub struct ChainWorkerServiceState<W> {
    #[allow(unused)]
    shared: Arc<Mutex<WorkerShared>>,

    #[allow(unused)]
    params: Arc<Params>,

    context: W,
    chain_exec: ChainExecutor,
    exec_ctl_handle: ExecCtlHandle,
    cur_tip: L2BlockCommitment,
    last_finalized_epoch: Option<EpochCommitment>,
    status_channel: StatusChannel,
    runtime_handle: Handle,
    initialized: bool,
}

impl<W: WorkerContext + Send + Sync + 'static> ChainWorkerServiceState<W> {
    pub(crate) fn new(
        shared: Arc<Mutex<WorkerShared>>,
        context: W,
        params: Arc<Params>,
        exec_ctl_handle: ExecCtlHandle,
        status_channel: StatusChannel,
        runtime_handle: Handle,
    ) -> Self {
        let rollup_params = params.rollup.clone();
        Self {
            shared,
            params,
            context,
            chain_exec: ChainExecutor::new(rollup_params),
            exec_ctl_handle,
            cur_tip: L2BlockCommitment::new(0, L2BlockId::default()),
            last_finalized_epoch: None,
            status_channel,
            runtime_handle,
            initialized: false,
        }
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn check_initialized(&self) -> WorkerResult<()> {
        if !self.is_initialized() {
            Err(WorkerError::NotInitialized)
        } else {
            Ok(())
        }
    }

    fn wait_for_genesis_and_resolve_tip(&self) -> WorkerResult<L2BlockCommitment> {
        info!("waiting until genesis");

        let init_state = self
            .runtime_handle
            .block_on(self.status_channel.wait_until_genesis())
            .map_err(|_| WorkerError::ShutdownBeforeGenesis)?;

        let cur_tip = match init_state.get_declared_final_epoch().cloned() {
            Some(epoch) => epoch.to_block_commitment(),
            None => L2BlockCommitment::new(
                0,
                *init_state.sync().expect("after genesis").genesis_blkid(),
            ),
        };

        Ok(cur_tip)
    }

    fn initialize_with_tip(&mut self, cur_tip: L2BlockCommitment) -> anyhow::Result<()> {
        let blkid = *cur_tip.blkid();
        info!(%blkid, "initializing chain worker");

        self.cur_tip = cur_tip;
        self.initialized = true;

        Ok(())
    }

    /// Prepares context for a block we're about to execute.
    fn prepare_block_context<'w>(
        &'w self,
        _l2bc: &L2BlockCommitment,
    ) -> WorkerResult<WorkerExecCtxImpl<'w, W>> {
        Ok(WorkerExecCtxImpl {
            worker_context: &self.context,
        })
    }

    fn try_exec_block(&mut self, block: &L2BlockCommitment) -> WorkerResult<()> {
        self.check_initialized()?;

        let context = &self.context;
        let chain_exec = &self.chain_exec;

        let blkid = block.blkid();
        debug!(%blkid, "Trying to execute block");

        let bundle = context
            .fetch_block(block.blkid())?
            .ok_or(WorkerError::MissingL2Block(*block.blkid()))?;

        let is_epoch_terminal = !bundle.body().l1_segment().new_manifests().is_empty();

        let parent_blkid = bundle.header().header().parent();
        let parent_header = context
            .fetch_header(parent_blkid)?
            .ok_or(WorkerError::MissingL2Block(*parent_blkid))?;

        self.exec_ctl_handle
            .try_exec_el_payload_blocking(*block)
            .map_err(|_| WorkerError::InvalidExecPayload(*block))?;

        let header_ctx = strata_chaintsn::context::L2HeaderAndParent::new(
            bundle.header().header().clone(),
            *parent_blkid,
            parent_header,
        );

        let exec_ctx = self.prepare_block_context(block)?;

        let output = chain_exec.verify_block(&header_ctx, bundle.body(), &exec_ctx)?;

        if is_epoch_terminal {
            debug!(%is_epoch_terminal);
            self.handle_complete_epoch(block.blkid(), bundle.block(), &output)?;
        }

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
        let context = &self.context;

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

        let summary = EpochSummary::new(
            prev_epoch_idx,
            terminal,
            prev_terminal,
            new_l1_block,
            *epoch_final_state,
        );

        debug!(?summary, "completed chain epoch");
        context.store_summary(summary)?;

        Ok(())
    }

    /// Updates the current tip as managed by the worker.  This does not persist
    /// in the client's database necessarily.
    fn update_cur_tip(&mut self, tip: L2BlockCommitment) -> WorkerResult<()> {
        self.cur_tip = tip;

        self.exec_ctl_handle
            .update_safe_tip_blocking(tip)
            .map_err(WorkerError::ExecEnvEngine)?;

        Ok(())
    }

    fn finalize_epoch(&mut self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.exec_ctl_handle
            .update_finalized_tip_blocking(epoch.to_block_commitment())
            .map_err(WorkerError::ExecEnvEngine)?;

        self.last_finalized_epoch = Some(epoch);

        Ok(())
    }
}

impl<W: WorkerContext + Send + Sync + 'static> ServiceState for ChainWorkerServiceState<W> {
    fn name(&self) -> &str {
        "chain_worker"
    }
}

/// Status information for the chain worker service.
#[derive(Clone, Debug, Serialize)]
pub struct ChainWorkerStatus {
    pub is_initialized: bool,
    pub cur_tip: L2BlockCommitment,
    pub last_finalized_epoch: Option<EpochCommitment>,
}
