//! Service state for the chain worker.

use std::sync::Arc;

use strata_checkpoint_types::EpochSummary;
use strata_identifiers::OLBlockCommitment;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_support_types::{IndexerState, IndexerWrites, WriteTrackingState};
use strata_ol_state_types::OLState;
use strata_ol_stf::verify_block;
use strata_params::Params;
use strata_primitives::{epoch::EpochCommitment, l1::L1BlockCommitment};
use strata_service::ServiceState;
use strata_status::StatusChannel;
use tokio::runtime::Handle;
use tracing::*;

use crate::{
    errors::{WorkerError, WorkerResult},
    output::OLBlockExecutionOutput,
    traits::WorkerContext,
};

/// Service state for the chain worker.
///
/// NOTE: Ideally, static dependencies like `context`, `runtime_handle`, etc. would live
/// in the Service struct rather than State. However, the current service framework doesn't
/// support this pattern. This should be refactored when the framework is updated.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug impl"
)]
pub struct ChainWorkerServiceState<W> {
    /// Parameters for the chain.
    #[expect(unused, reason = "params will be used for chain configuration")]
    params: Arc<Params>,

    /// Context for the worker (database access layer).
    context: W,

    /// Current tip commitment.
    pub(crate) cur_tip: OLBlockCommitment,

    /// Last finalized epoch, if any.
    pub(crate) last_finalized_epoch: Option<EpochCommitment>,

    /// Status channel for the worker.
    status_channel: StatusChannel,

    /// Runtime handle for the worker.
    runtime_handle: Handle,

    /// Whether the worker has been initialized.
    initialized: bool,
}

impl<W: WorkerContext + Send + Sync + 'static> ChainWorkerServiceState<W> {
    /// Creates a new chain worker service state.
    pub fn new(
        context: W,
        params: Arc<Params>,
        status_channel: StatusChannel,
        runtime_handle: Handle,
    ) -> Self {
        Self {
            params,
            context,
            cur_tip: OLBlockCommitment::null(),
            last_finalized_epoch: None,
            status_channel,
            runtime_handle,
            initialized: false,
        }
    }

    /// Returns whether the worker has been initialized.
    pub(crate) fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn check_initialized(&self) -> WorkerResult<()> {
        if !self.is_initialized() {
            Err(WorkerError::NotInitialized)
        } else {
            Ok(())
        }
    }

    /// Waits for genesis and resolves the initial tip commitment.
    pub(crate) fn wait_for_genesis_and_resolve_tip(&self) -> WorkerResult<OLBlockCommitment> {
        info!("waiting until genesis");

        let init_state = self
            .runtime_handle
            .block_on(self.status_channel.wait_until_genesis())
            .map_err(|_| WorkerError::ShutdownBeforeGenesis)?;

        let cur_tip = match init_state.get_declared_final_epoch() {
            Some(epoch) => {
                // Convert from L2BlockCommitment to OLBlockCommitment
                let l2bc = epoch.to_block_commitment();
                OLBlockCommitment::new(l2bc.slot(), *l2bc.blkid())
            }
            None => {
                // Get genesis block ID by fetching the first block at slot 0
                let genesis_block_ids = self.context.fetch_blocks_at_slot(0)?;
                let genesis_blkid = *genesis_block_ids
                    .first()
                    .ok_or(WorkerError::MissingGenesisBlock)?;
                OLBlockCommitment::new(0, genesis_blkid)
            }
        };

        Ok(cur_tip)
    }

    /// Initializes the worker with the given tip commitment.
    pub(crate) fn initialize_with_tip(&mut self, cur_tip: OLBlockCommitment) -> anyhow::Result<()> {
        let blkid = *cur_tip.blkid();
        info!(%blkid, "initializing chain worker");

        self.cur_tip = cur_tip;
        self.initialized = true;

        Ok(())
    }

    /// Tries to execute a block using the new OL STF.
    pub(crate) fn try_exec_block(
        &mut self,
        block_commitment: &OLBlockCommitment,
    ) -> WorkerResult<()> {
        self.check_initialized()?;

        let blkid = block_commitment.blkid();
        debug!(%blkid, "Trying to execute block");

        // Fetch block and parent context
        let (block, parent_header, parent_commitment) =
            self.fetch_block_with_parent(block_commitment)?;

        // Execute STF and get output
        let output = self.execute_stf(&block, parent_header.as_ref(), parent_commitment)?;

        // Persist results
        self.persist_execution_output(*block_commitment, &output)?;

        // Handle epoch terminal if needed
        if block.header().is_terminal() {
            self.handle_complete_epoch(&block, &output)?;
        }

        Ok(())
    }

    /// Fetches a block and its parent header from the context.
    ///
    /// Returns the block, optional parent header, and parent commitment.
    fn fetch_block_with_parent(
        &self,
        block_commitment: &OLBlockCommitment,
    ) -> WorkerResult<(OLBlock, Option<OLBlockHeader>, OLBlockCommitment)> {
        let blkid = block_commitment.blkid();

        let block = self
            .context
            .fetch_block(blkid)?
            .ok_or(WorkerError::MissingOLBlock(*blkid))?;

        let parent_blkid = block.header().parent_blkid();
        let parent_commitment = if parent_blkid.is_null() {
            OLBlockCommitment::null()
        } else {
            // We need to figure out the parent slot. For now, assume slot-1.
            // TODO: Properly track parent slot
            let parent_slot = block.header().slot().saturating_sub(1);
            OLBlockCommitment::new(parent_slot, *parent_blkid)
        };

        let parent_header = if parent_commitment.is_null() {
            None
        } else {
            Some(
                self.context
                    .fetch_header(parent_commitment.blkid())?
                    .ok_or(WorkerError::MissingOLBlock(*parent_commitment.blkid()))?,
            )
        };

        Ok((block, parent_header, parent_commitment))
    }

    /// Executes the STF on a block and returns the execution output.
    ///
    /// This fetches parent state, builds the state stack, runs verification,
    /// and extracts the resulting write batch and indexer writes.
    fn execute_stf(
        &self,
        block: &OLBlock,
        parent_header: Option<&OLBlockHeader>,
        parent_commitment: OLBlockCommitment,
    ) -> WorkerResult<OLBlockExecutionOutput> {
        // Fetch parent state
        let parent_state = self
            .context
            .fetch_ol_state(parent_commitment)?
            .ok_or(WorkerError::MissingPreState(parent_commitment))?;

        // Execute and extract outputs
        let (write_batch, indexer_writes) =
            Self::run_stf_verification(&parent_state, block, parent_header)?;

        // Use the state root from the header (verify_block validated it)
        let computed_state_root = *block.header().state_root();
        let logs = Vec::new(); // TODO: Collect logs from execution context when available

        Ok(OLBlockExecutionOutput::new(
            computed_state_root,
            logs,
            write_batch,
            indexer_writes,
        ))
    }

    /// Runs the STF verification on a block.
    ///
    /// This is a pure function that builds the state stack and executes the STF.
    fn run_stf_verification(
        parent_state: &OLState,
        block: &OLBlock,
        parent_header: Option<&OLBlockHeader>,
    ) -> WorkerResult<(
        strata_ol_state_types::WriteBatch<strata_ol_state_types::NativeAccountState>,
        IndexerWrites,
    )> {
        // Build the state stack: IndexerState<WriteTrackingState<&OLState>>
        let tracking_state = WriteTrackingState::new_from_state(parent_state);
        let mut indexer_state = IndexerState::new(tracking_state);

        // Execute using new OL STF
        verify_block(
            &mut indexer_state,
            block.header(),
            parent_header.cloned(),
            block.body(),
        )?;

        // Extract outputs
        let (tracking_state, indexer_writes) = indexer_state.into_parts();
        let write_batch = tracking_state.into_batch();

        Ok((write_batch, indexer_writes))
    }

    /// Persists the execution output to storage.
    fn persist_execution_output(
        &self,
        block_commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        self.context.store_block_output(block_commitment, output)?;
        self.context
            .store_auxiliary_data(block_commitment, output.indexer_writes())?;
        Ok(())
    }

    /// Takes the block and post-state and inserts database entries to reflect
    /// the epoch being finished on-chain.
    fn handle_complete_epoch(
        &mut self,
        block: &OLBlock,
        last_block_output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        // Get epoch info from the write batch
        let epochal = last_block_output.write_batch().epochal();
        let prev_epoch_idx = epochal.cur_epoch();
        let prev_terminal = epochal.asm_recorded_epoch().to_block_commitment();

        let slot = block.header().slot();
        let terminal = OLBlockCommitment::new(slot, block.header().compute_blkid());

        // Get L1 info from the write batch (epochal state has latest L1 after manifest sealing)
        let new_tip_height = epochal.last_l1_height().into();
        let new_tip_blkid = epochal.last_l1_blkid();
        let new_l1_block = L1BlockCommitment::from_height_u64(new_tip_height, *new_tip_blkid)
            .expect("valid height");

        let epoch_final_state = *last_block_output.computed_state_root();

        // terminal and prev_terminal are already OLBlockCommitment = L2BlockCommitment
        let terminal_l2 = terminal;
        let prev_terminal_l2 = prev_terminal;

        let summary = EpochSummary::new(
            prev_epoch_idx,
            terminal_l2,
            prev_terminal_l2,
            new_l1_block,
            epoch_final_state,
        );

        debug!(?summary, "completed chain epoch");
        self.context.store_summary(summary)?;

        Ok(())
    }

    /// Updates the current tip as managed by the worker.
    pub(crate) fn update_cur_tip(&mut self, tip: OLBlockCommitment) -> WorkerResult<()> {
        self.cur_tip = tip;
        Ok(())
    }

    /// Finalizes an epoch, merging write batches into finalized state.
    pub(crate) fn finalize_epoch(&mut self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.context.merge_finalized_epoch(&epoch)?;
        self.last_finalized_epoch = Some(epoch);
        Ok(())
    }
}

impl<W: WorkerContext + Send + Sync + 'static> ServiceState for ChainWorkerServiceState<W> {
    fn name(&self) -> &str {
        "chain_worker_new"
    }
}
