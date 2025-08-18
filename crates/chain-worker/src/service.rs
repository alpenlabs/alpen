//! Service framework integration for chain worker.

use std::sync::Arc;

use serde::Serialize;
use strata_chainexec::{ChainExecutor, ExecContext, ExecResult};
use strata_eectl::handle::ExecCtlHandle;
use strata_primitives::{params::Params, prelude::*};
use strata_service::{Response, Service, ServiceState, SyncService};
use strata_state::{block::L2Block, header::L2Header};
use strata_status::StatusChannel;
use tokio::{runtime::Handle, sync::Mutex};

use crate::{
    message::ChainWorkerMessage, WorkerContext, WorkerError, WorkerResult, WorkerShared,
    WorkerExecCtxImpl,
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
            current_tip: state.cur_tip,
            is_initialized: state.is_initialized(),
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
                if let Ok(mut guard) = completion.blocking_lock() {
                    if let Some(sender) = guard.take() {
                        let _ = sender.send(res);
                    }
                }
            }

            ChainWorkerMessage::UpdateSafeTip(l2bc, completion) => {
                let res = state.update_cur_tip(*l2bc);
                if let Ok(mut guard) = completion.blocking_lock() {
                    if let Some(sender) = guard.take() {
                        let _ = sender.send(res);
                    }
                }
            }

            ChainWorkerMessage::FinalizeEpoch(epoch, completion) => {
                let res = state.finalize_epoch(*epoch);
                if let Ok(mut guard) = completion.blocking_lock() {
                    if let Some(sender) = guard.take() {
                        let _ = sender.send(res);
                    }
                }
            }
        }

        Ok(Response::Continue)
    }
}

/// Service state for the chain worker.
pub struct ChainWorkerServiceState<W> {
    shared: Arc<Mutex<WorkerShared>>,
    context: Option<W>,
    chain_exec: Option<ChainExecutor>,
    exec_ctl_handle: Option<ExecCtlHandle>,
    cur_tip: L2BlockCommitment,
    params: Arc<Params>,
    status_channel: StatusChannel,
    runtime_handle: Handle,
    initialized: bool,
}

impl<W: WorkerContext + Send + Sync + 'static> ChainWorkerServiceState<W> {
    pub fn new(
        shared: Arc<Mutex<WorkerShared>>,
        context: W,
        params: Arc<Params>,
        exec_ctl_handle: ExecCtlHandle,
        status_channel: StatusChannel,
        runtime_handle: Handle,
    ) -> Self {
        Self {
            shared,
            context: Some(context),
            chain_exec: None,
            exec_ctl_handle: Some(exec_ctl_handle),
            cur_tip: L2BlockCommitment::new(0, L2BlockId::default()),
            params,
            status_channel,
            runtime_handle,
            initialized: false,
        }
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn wait_for_genesis_and_resolve_tip(&self) -> WorkerResult<L2BlockCommitment> {
        tracing::info!("waiting until genesis");

        let init_state = self
            .runtime_handle
            .block_on(self.status_channel.wait_until_genesis())
            .map_err(|e| WorkerError::Unexpected(format!("failed to wait for genesis: {e}")))?;

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
        tracing::info!(%blkid, "starting chain worker");

        self.cur_tip = cur_tip;
        self.chain_exec = Some(ChainExecutor::new(self.params.rollup().clone()));
        self.initialized = true;

        Ok(())
    }

    fn try_exec_block(&mut self, block: &L2BlockCommitment) -> WorkerResult<()> {
        if !self.is_initialized() {
            return Err(WorkerError::Unexpected("worker not initialized".to_string()));
        }

        let context = self
            .context
            .as_ref()
            .ok_or(WorkerError::Unexpected("missing context".to_string()))?;
        let chain_exec = self
            .chain_exec
            .as_ref()
            .ok_or(WorkerError::Unexpected("missing chain executor".to_string()))?;

        let blkid = block.blkid();
        tracing::debug!(%blkid, "Trying to execute block");

        let bundle = context
            .fetch_block(block.blkid())?
            .ok_or(WorkerError::MissingL2Block(*block.blkid()))?;

        let is_epoch_terminal = !bundle.body().l1_segment().new_manifests().is_empty();

        let parent_blkid = bundle.header().header().parent();
        let parent_header = context
            .fetch_header(parent_blkid)?
            .ok_or(WorkerError::MissingL2Block(*parent_blkid))?;

        if let Some(exec_ctl) = &self.exec_ctl_handle {
            exec_ctl
                .try_exec_el_payload_blocking(*block)
                .map_err(|_| WorkerError::InvalidExecPayload(*block))?;
        }

        let header_ctx = strata_chaintsn::context::L2HeaderAndParent::new(
            bundle.header().header().clone(),
            *parent_blkid,
            parent_header,
        );

        let exec_ctx = WorkerExecCtxImpl {
            worker_context: context,
        };

        let output = chain_exec.verify_block(&header_ctx, bundle.body(), &exec_ctx)?;

        if is_epoch_terminal {
            tracing::debug!(%is_epoch_terminal);
            self.handle_complete_epoch(block.blkid(), bundle.block(), &output)?;
        }

        context.store_block_output(block.blkid(), &output)?;

        Ok(())
    }

    fn handle_complete_epoch(
        &mut self,
        blkid: &L2BlockId,
        block: &L2Block,
        last_block_output: &strata_chainexec::BlockExecutionOutput,
    ) -> WorkerResult<()> {
        let context = self
            .context
            .as_ref()
            .ok_or(WorkerError::Unexpected("missing context".to_string()))?;

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

        let summary = strata_primitives::batch::EpochSummary::new(
            prev_epoch_idx,
            terminal,
            prev_terminal,
            new_l1_block,
            *epoch_final_state,
        );

        tracing::debug!(?summary, "completed chain epoch");
        context.store_summary(summary)?;

        Ok(())
    }

    fn update_cur_tip(&mut self, tip: L2BlockCommitment) -> WorkerResult<()> {
        self.cur_tip = tip;

        if let Some(exec_ctl) = &self.exec_ctl_handle {
            exec_ctl
                .update_safe_tip_blocking(tip)
                .map_err(WorkerError::ExecEnvEngine)?;
        }

        Ok(())
    }

    fn finalize_epoch(&mut self, epoch: EpochCommitment) -> WorkerResult<()> {
        if let Some(exec_ctl) = &self.exec_ctl_handle {
            exec_ctl
                .update_finalized_tip_blocking(epoch.to_block_commitment())
                .map_err(WorkerError::ExecEnvEngine)?;
        }

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
    pub current_tip: L2BlockCommitment,
    pub is_initialized: bool,
}