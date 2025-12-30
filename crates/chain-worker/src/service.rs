//! Service framework integration for chain worker.

use std::sync::Arc;

use serde::Serialize;
use strata_checkpoint_types::EpochSummary;
use strata_eectl::handle::ExecCtlHandle;
use strata_identifiers::OLBlockCommitment;
use strata_params::Params;
use strata_primitives::prelude::*;
use strata_service::{Response, Service, ServiceState, SyncService};
use strata_status::StatusChannel;
use tokio::{runtime::Handle, sync::Mutex};
use tracing::*;

use crate::{
    OLBlockExecutionOutput,
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
            ChainWorkerMessage::TryExecBlock(commitment, completion) => {
                let res = state.try_exec_block(commitment);
                completion.send_blocking(res);
            }

            ChainWorkerMessage::UpdateSafeTip(commitment, completion) => {
                let res = state.update_cur_tip(*commitment);
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
    #[expect(unused, reason = "will be used later")]
    shared: Arc<Mutex<WorkerShared>>,

    #[expect(unused, reason = "will be used for epoch handling")]
    params: Arc<Params>,

    context: W,
    exec_ctl_handle: ExecCtlHandle,
    cur_tip: OLBlockCommitment,
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
        Self {
            shared,
            params,
            context,
            exec_ctl_handle,
            cur_tip: OLBlockCommitment::null(),
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

    fn wait_for_genesis_and_resolve_tip(&self) -> WorkerResult<OLBlockCommitment> {
        info!("waiting until genesis");

        let init_state = self
            .runtime_handle
            .block_on(self.status_channel.wait_until_genesis())
            .map_err(|_| WorkerError::ShutdownBeforeGenesis)?;

        // TODO: Properly convert the declared final epoch to OLBlockCommitment
        // For now, we get the genesis block at slot 0
        let _declared_epoch = init_state.get_declared_final_epoch();

        // Get genesis block ID by fetching the first block at slot 0
        let genesis_block_ids = self.context.fetch_blocks_at_slot(0)?;
        let genesis_blkid = *genesis_block_ids
            .first()
            .ok_or(WorkerError::MissingGenesisBlock)?;

        Ok(OLBlockCommitment::new(0, genesis_blkid))
    }

    fn initialize_with_tip(&mut self, cur_tip: OLBlockCommitment) -> anyhow::Result<()> {
        let blkid = *cur_tip.blkid();
        info!(%blkid, "initializing chain worker");

        self.cur_tip = cur_tip;
        self.initialized = true;

        Ok(())
    }

    fn try_exec_block(&mut self, block_commitment: &OLBlockCommitment) -> WorkerResult<()> {
        self.check_initialized()?;

        let blkid = block_commitment.blkid();
        debug!(%blkid, "Trying to execute block");

        // 1. Fetch the block
        let block = self
            .context
            .fetch_block(blkid)?
            .ok_or(WorkerError::MissingBlock(*blkid))?;

        // 2. Fetch parent header
        let parent_blkid = block.header().parent_blkid();
        let parent_header = self.context.fetch_header(parent_blkid)?;

        // 3. Get parent state and create layered accessor
        let parent_commitment = OLBlockCommitment::new(block.header().slot() - 1, *parent_blkid);
        let parent_state = self
            .context
            .fetch_ol_state(parent_commitment)?
            .ok_or(WorkerError::MissingOLState(parent_commitment))?;

        // 4. Execute EE payload (this validates the EL block)
        self.exec_ctl_handle
            .try_exec_el_payload_blocking(*block_commitment)
            .map_err(|_| WorkerError::InvalidExecPayload(*block_commitment))?;

        // 5. Execute using new OL STF with layered state accessor
        #[rustfmt::skip] // Ignore formatting for the block comment for the TODO block.
        // TODO: Implement the actual execution using:
        // let tracking = WriteTrackingState::new_from_state(&parent_state);
        // let mut state = IndexerState::new(tracking);
        // strata_ol_stf::verify_block(&mut state, block.header(), parent_header.as_ref(), block.body())?;
        // let (tracking_state, indexer_writes) = state.into_parts();
        // let write_batch = tracking_state.into_batch();
        //
        // For now, this is a stub that creates placeholder output.
        let _ = (&parent_state, &parent_header);

        // Placeholder: In the real implementation, these would come from execution
        // using strata_ol_stf::verify_block() with IndexerState<WriteTrackingState<&OLState>>
        let computed_state_root = strata_identifiers::Hash::zero();
        let logs = Vec::new();
        // Create a minimal write batch from the parent state
        let write_batch = strata_ol_state_types::WriteBatch::new_from_state(&parent_state);
        let indexer_writes = strata_ol_state_support_types::IndexerWrites::new();

        let output = OLBlockExecutionOutput::new(
            computed_state_root,
            logs,
            write_batch,
            indexer_writes.clone(),
        );

        // 6. Check if epoch terminal
        let is_epoch_terminal = block.header().is_terminal();
        if is_epoch_terminal {
            debug!(%is_epoch_terminal, "block is epoch terminal");
            self.handle_complete_epoch(block_commitment, &block, &output)?;
        }

        // 7. Persist outputs
        self.context
            .store_block_output(*block_commitment, &output)?;
        self.context
            .store_auxiliary_data(*block_commitment, &indexer_writes)?;

        Ok(())
    }

    /// Takes the block and post-state and inserts database entries to reflect
    /// the epoch being finished on-chain.
    fn handle_complete_epoch(
        &mut self,
        block_commitment: &OLBlockCommitment,
        block: &strata_ol_chain_types_new::OLBlock,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        let header = block.header();

        // Get the terminal block information
        let slot = header.slot();
        let blkid = *block_commitment.blkid();
        let terminal = OLBlockCommitment::new(slot, blkid);

        // Get previous terminal from the epoch we're finishing
        // TODO: Extract this from the state properly
        let prev_epoch_idx = header.epoch();
        let prev_terminal = OLBlockCommitment::null(); // Placeholder

        // Get the L1 block information from the body's L1 update
        // TODO: Extract from body properly
        let new_l1_block = L1BlockCommitment::from_height_u64(
            0,
            L1BlockId::from(strata_identifiers::Buf32::zero()),
        )
        .expect("valid height");

        let epoch_final_state = *output.computed_state_root();

        let summary = EpochSummary::new(
            prev_epoch_idx,
            terminal,
            prev_terminal,
            new_l1_block,
            epoch_final_state,
        );

        debug!(?summary, "completed chain epoch");
        self.context.store_summary(summary)?;

        Ok(())
    }

    /// Updates the current tip as managed by the worker.
    fn update_cur_tip(&mut self, tip: OLBlockCommitment) -> WorkerResult<()> {
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
    pub cur_tip: OLBlockCommitment,
    pub last_finalized_epoch: Option<EpochCommitment>,
}
