//! Service state for the chain worker.
//!
//! This module contains the state management for the chain worker service.
//! The state is internally organized into:
//! - [`ChainWorkerDeps`]: Static dependencies (context, params, runtime handles)
//! - [`ChainWorkerMutableState`]: Actual mutable state (tip, epoch info, etc.)
//!
//! This separation makes it clear which parts are actual "state" vs dependencies,
//! even though both must live in [`ChainWorkerServiceState`] due to the current
//! service framework design.

use strata_checkpoint_types::EpochSummary;
use strata_identifiers::OLBlockCommitment;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_support_types::{IndexerState, IndexerWrites, WriteTrackingState};
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
use strata_ol_stf::verify_block_with_root_fn;
use strata_primitives::{epoch::EpochCommitment, l1::L1BlockCommitment};
use strata_service::ServiceState;
use tracing::*;

use crate::{
    ChainWorkerContextImpl,
    errors::{WorkerError, WorkerResult},
    output::OLBlockExecutionOutput,
    traits::ChainWorkerContext,
};

/// Mutable state for the chain worker.
///
/// This contains the actual "state" - data that changes during the worker's
/// operation and represents the current processing position.
#[derive(Debug)]
struct ChainWorkerMutableState {
    /// Current tip commitment.
    cur_tip: OLBlockCommitment,

    /// Last finalized epoch, if any.
    last_finalized_epoch: Option<EpochCommitment>,

    /// Whether the worker has been initialized.
    initialized: bool,
}

impl Default for ChainWorkerMutableState {
    fn default() -> Self {
        Self {
            cur_tip: OLBlockCommitment::null(),
            last_finalized_epoch: None,
            initialized: false,
        }
    }
}

/// Service state for the chain worker.
///
/// This combines static dependencies with mutable state. The separation is
/// internal to make the code clearer about what is actual "state" vs what
/// are just dependencies needed for operations.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug impl"
)]
pub struct ChainWorkerServiceState {
    /// Static dependencies.
    ctx: ChainWorkerContextImpl,

    /// Mutable state.
    state: ChainWorkerMutableState,
}

impl ChainWorkerServiceState {
    /// Creates a new chain worker service state.
    pub fn new(ctx: ChainWorkerContextImpl) -> Self {
        Self {
            ctx,
            state: ChainWorkerMutableState::default(),
        }
    }

    /// Returns whether the worker has been initialized.
    pub(crate) fn is_initialized(&self) -> bool {
        self.state.initialized
    }

    fn check_initialized(&self) -> WorkerResult<()> {
        if !self.is_initialized() {
            Err(WorkerError::NotInitialized)
        } else {
            Ok(())
        }
    }

    /// Returns the current tip commitment.
    pub(crate) fn cur_tip(&self) -> OLBlockCommitment {
        self.state.cur_tip
    }

    /// Returns the last finalized epoch, if any.
    pub(crate) fn last_finalized_epoch(&self) -> Option<EpochCommitment> {
        self.state.last_finalized_epoch
    }

    /// Waits for genesis and resolves the initial tip commitment.
    ///
    /// This first checks the database for an existing chain tip (highest executed block).
    /// If found, it resumes from there. Otherwise, it waits for genesis and starts fresh.
    pub(crate) fn wait_for_genesis_and_resolve_tip(&self) -> WorkerResult<OLBlockCommitment> {
        // First, check if we have an existing chain tip in the database.
        // This allows us to resume from where we left off after a restart,
        // including unfinalized blocks.
        if let Some(db_tip) = self.ctx.fetch_chain_tip()? {
            info!(slot = db_tip.slot(), %db_tip, "resuming from database chain tip");
            return Ok(db_tip);
        }

        // No existing chain - wait for genesis
        info!("waiting until genesis");

        let _init_state = self
            .ctx
            .handle()
            .block_on(self.ctx.status_channel().wait_until_genesis())
            .map_err(|_| WorkerError::ShutdownBeforeGenesis)?;

        // Start from genesis block
        let genesis_block_ids = self.ctx.fetch_blocks_at_slot(0)?;
        let genesis_blkid = *genesis_block_ids
            .first()
            .ok_or(WorkerError::MissingGenesisBlock)?;

        Ok(OLBlockCommitment::new(0, genesis_blkid))
    }

    /// Initializes the worker with the given tip commitment.
    pub(crate) fn initialize_with_tip(&mut self, cur_tip: OLBlockCommitment) -> anyhow::Result<()> {
        let blkid = *cur_tip.blkid();
        info!(%blkid, "initializing chain worker");

        self.state.cur_tip = cur_tip;
        self.state.initialized = true;

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
            .ctx
            .fetch_block(blkid)?
            .ok_or(WorkerError::MissingOLBlock(*blkid))?;

        let parent_blkid = block.header().parent_blkid();
        let parent_commitment = if parent_blkid.is_null() {
            OLBlockCommitment::null()
        } else {
            // Parent slot is the block's slot - 1.
            let parent_slot = block.header().slot().saturating_sub(1);
            OLBlockCommitment::new(parent_slot, *parent_blkid)
        };

        let parent_header = if parent_commitment.is_null() {
            None
        } else {
            Some(
                self.ctx
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
            .ctx
            .fetch_ol_state(parent_commitment)?
            .ok_or(WorkerError::MissingPreState(parent_commitment))?;

        // Execute and extract outputs
        let (write_batch, indexer_writes) =
            Self::run_stf_verification(&parent_state, block, parent_header)?;

        // Use the state root from the header (verify_block validated it).
        // Note: logs are validated internally by verify_block via the logs_root commitment.
        let computed_state_root = *block.header().state_root();

        Ok(OLBlockExecutionOutput::new(
            computed_state_root,
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
    ) -> WorkerResult<(WriteBatch<OLAccountState>, IndexerWrites)> {
        // Build the state stack: IndexerState<WriteTrackingState<&OLState>>
        let tracking_state = WriteTrackingState::new_from_state(parent_state);
        let mut indexer_state = IndexerState::new(tracking_state);

        // Execute using new OL STF, providing materialized state root computation
        // since WriteTrackingState::compute_state_root() is unsupported.
        verify_block_with_root_fn(
            &mut indexer_state,
            block.header(),
            parent_header.cloned(),
            block.body(),
            |s| {
                s.inner()
                    .compute_state_root_materialized()
                    .map_err(Into::into)
            },
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
        self.ctx.store_block_output(block_commitment, output)?;
        self.ctx
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
        // Use the block header epoch - this is the epoch being completed.
        // Note: The write batch contains POST-manifest state where cur_epoch is already
        // advanced. The header epoch is set during block assembly and doesn't change.
        let completed_epoch = block.header().epoch();

        let slot = block.header().slot();
        let terminal = OLBlockCommitment::new(slot, block.header().compute_blkid());

        // Get previous terminal from storage.
        // Note: Epoch 0 (genesis) is created by genesis initialization, not chain-worker.
        // Chain-worker starts processing from slot 1, so completed_epoch >= 1 is guaranteed.
        let prev_summaries = self.ctx.fetch_epoch_summaries(completed_epoch - 1)?;
        let prev_terminal = prev_summaries
            .first()
            .map(|s| *s.terminal())
            .unwrap_or(OLBlockCommitment::null());

        // Get L1 info from the write batch (epochal state has latest L1 after manifest sealing)
        let epochal = last_block_output.write_batch().epochal();
        let new_tip_height = epochal.last_l1_height().into();
        let new_tip_blkid = epochal.last_l1_blkid();
        let new_l1_block = L1BlockCommitment::from_height_u64(new_tip_height, *new_tip_blkid)
            .expect("valid height");

        let epoch_final_state = *last_block_output.computed_state_root();

        let summary = EpochSummary::new(
            completed_epoch,
            terminal,
            prev_terminal,
            new_l1_block,
            epoch_final_state,
        );

        debug!(?summary, "completed chain epoch");
        self.ctx.store_summary(summary)?;

        Ok(())
    }

    /// Updates the current tip as managed by the worker.
    pub(crate) fn update_cur_tip(&mut self, tip: OLBlockCommitment) -> WorkerResult<()> {
        self.state.cur_tip = tip;
        Ok(())
    }

    /// Finalizes an epoch, merging write batches into finalized state.
    pub(crate) fn finalize_epoch(&mut self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.ctx.merge_finalized_epoch(&epoch)?;
        self.state.last_finalized_epoch = Some(epoch);
        Ok(())
    }
}

impl ServiceState for ChainWorkerServiceState {
    fn name(&self) -> &str {
        "chain_worker_new"
    }
}
