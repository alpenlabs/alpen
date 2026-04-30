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
use strata_ol_state_support_types::{
    IndexerState, IndexerWrites, MemoryStateBaseLayer, WriteTrackingState,
};
use strata_ol_state_types::{IStateBatchApplicable, OLAccountState, OLState, WriteBatch};
use strata_ol_stf::verify_block;
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

        // Execute STF and get output and new state
        let (output, new_state) =
            self.execute_stf(&block, parent_header.as_ref(), parent_commitment)?;

        // Handle epoch terminal if needed
        debug!(slot=%block.header().slot(), is_terminal=%block.header().is_terminal(), "Checking if block is terminal");
        if block.header().is_terminal() {
            self.handle_complete_epoch(&block, &output, &new_state)?;
            // Send the epoch commitment to receiver
            // TODO: it seems to be done for each block at the moment. Ideally we would do it just
            // here.
        }

        // Persist results (including the full state)
        self.persist_execution_output(&block, *block_commitment, &output, new_state)?;

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
    ) -> WorkerResult<(OLBlockExecutionOutput, OLState)> {
        // Fetch parent state and wrap in MemoryStateBaseLayer for IStateAccessor
        let parent_state_raw = self
            .ctx
            .fetch_ol_state(parent_commitment)?
            .ok_or(WorkerError::MissingPreState(parent_commitment))?;
        let parent_state = MemoryStateBaseLayer::new(parent_state_raw);

        // Execute and extract outputs
        let (write_batch, indexer_writes) =
            Self::run_stf_verification(&parent_state, block, parent_header)?;

        // Apply write batch to parent state to get new state
        let mut new_state = parent_state;
        new_state
            .apply_write_batch(write_batch.clone())
            .map_err(|e| WorkerError::Unexpected(format!("Failed to apply write batch: {}", e)))?;
        let new_state = new_state.into_inner();

        // Use the state root from the header (verify_block validated it).
        // Note: logs are validated internally by verify_block via the logs_root commitment.
        let computed_state_root = *block.header().state_root();

        Ok((
            OLBlockExecutionOutput::new(computed_state_root, write_batch, indexer_writes),
            new_state,
        ))
    }

    /// Runs the STF verification on a block.
    ///
    /// This is a pure function that builds the state stack and executes the STF.
    fn run_stf_verification(
        parent_state: &MemoryStateBaseLayer,
        block: &OLBlock,
        parent_header: Option<&OLBlockHeader>,
    ) -> WorkerResult<(WriteBatch<OLAccountState>, IndexerWrites)> {
        // Build the state stack: IndexerState<WriteTrackingState<&MemoryStateBaseLayer>>
        let tracking_state = WriteTrackingState::new_empty(parent_state);
        let mut indexer_state = IndexerState::new(tracking_state);

        verify_block(
            &mut indexer_state,
            block.header(),
            parent_header,
            block.body(),
        )?;

        // Extract outputs
        let (tracking_state, indexer_writes) = indexer_state.into_parts();
        let write_batch: WriteBatch<OLAccountState> = tracking_state.into_batch();

        Ok((write_batch, indexer_writes))
    }

    /// Persists the execution output and state to storage.
    fn persist_execution_output(
        &self,
        block: &OLBlock,
        block_commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
        new_state: OLState,
    ) -> WorkerResult<()> {
        // Store the write batch
        self.ctx
            .store_block_output(block, block_commitment, output)?;

        // Store the full toplevel state
        self.ctx.store_toplevel_state(block_commitment, new_state)?;

        // Store auxiliary data
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
        new_state: &OLState,
    ) -> WorkerResult<()> {
        let completed_epoch = block.header().epoch();

        // Get previous terminal from storage.
        // Note: Epoch 0 (genesis) is created by genesis initialization, not chain-worker.
        // Chain-worker starts processing from slot 1, so completed_epoch >= 1 is guaranteed.
        let prev_summaries = self.ctx.fetch_epoch_summaries(completed_epoch - 1)?;
        let prev_terminal = prev_summaries
            .first()
            .map(|s| *s.terminal())
            .unwrap_or(OLBlockCommitment::null());

        let summary =
            build_epoch_summary(block.header(), last_block_output, new_state, prev_terminal);

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

/// Builds the [`EpochSummary`] for a completed epoch from the terminal block,
/// its execution output, and the post-state.
///
/// L1 info is sourced from the post-state's epochal state rather than the
/// write batch, since the batch only carries diffs and may not contain a
/// `last_l1_*` update if the terminal block introduced no new manifests.
fn build_epoch_summary(
    block_header: &OLBlockHeader,
    last_block_output: &OLBlockExecutionOutput,
    new_state: &OLState,
    prev_terminal: OLBlockCommitment,
) -> EpochSummary {
    let completed_epoch = block_header.epoch();
    let terminal = OLBlockCommitment::new(block_header.slot(), block_header.compute_blkid());

    // Read L1 info from the post-state. The write batch only stores diffs, so it
    // may not contain a last_l1_* update if the terminal block had no new manifests.
    let epoch_state = new_state.epoch_state();
    let new_l1_block =
        L1BlockCommitment::new(epoch_state.last_l1_height(), *epoch_state.last_l1_blkid());

    let epoch_final_state = *last_block_output.computed_state_root();

    EpochSummary::new(
        completed_epoch,
        terminal,
        prev_terminal,
        new_l1_block,
        epoch_final_state,
    )
}

#[cfg(test)]
mod tests {
    use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, L1Height, OLBlockId};
    use strata_ol_chain_types_new::{BlockFlags, OLBlockHeader};
    use strata_ol_state_support_types::IndexerWrites;
    use strata_ol_state_types::{
        OLAccountState, WriteBatch, test_utils::create_test_genesis_state,
    };

    use super::*;
    use crate::OLBlockExecutionOutput;

    /// Regression test for the panic
    /// `terminal block must have L1 height in write batch`.
    ///
    /// When a terminal block does not introduce any new ASM manifests, its
    /// [`WriteBatch`] does not contain `last_l1_height` / `last_l1_blkid`
    /// updates. The summary builder must therefore read L1 info from the
    /// post-state rather than the write batch.
    #[test]
    fn test_handle_write_batch_without_last_l1_change() {
        // Set up a post-state with a known L1 commitment, simulating L1 progress
        // recorded in earlier (non-terminal) blocks of the epoch.
        let mut new_state = create_test_genesis_state();
        let expected_height = L1Height::from(1234u32);
        let expected_blkid = L1BlockId::from(Buf32::from([7u8; 32]));
        let mut setup_batch: WriteBatch<OLAccountState> = WriteBatch::default();
        setup_batch.epochal_writes_mut().last_l1_height = Some(expected_height);
        setup_batch.epochal_writes_mut().last_l1_blkid = Some(expected_blkid);
        new_state
            .apply_write_batch(setup_batch)
            .expect("apply setup batch");

        // Terminal block's write batch carries no `last_l1_*` update — this is
        // the case that previously panicked.
        let terminal_batch: WriteBatch<OLAccountState> = WriteBatch::default();
        assert!(terminal_batch.epochal_writes().last_l1_height.is_none());
        assert!(terminal_batch.epochal_writes().last_l1_blkid.is_none());

        let state_root = Buf32::from([9u8; 32]);
        let output = OLBlockExecutionOutput::new(state_root, terminal_batch, IndexerWrites::new());

        let mut flags = BlockFlags::zero();
        flags.set_is_terminal(true);
        let header = OLBlockHeader::new(
            0,
            flags,
            10,
            5,
            OLBlockId::from(Buf32::zero()),
            Buf32::zero(),
            state_root,
            Buf32::zero(),
        );

        let summary = build_epoch_summary(&header, &output, &new_state, OLBlockCommitment::null());

        assert_eq!(
            summary.new_l1(),
            &L1BlockCommitment::new(expected_height, expected_blkid),
            "L1 commitment must come from post-state, not from the empty write batch"
        );
        assert_eq!(summary.epoch(), 5);
        assert_eq!(summary.final_state(), &state_root);
        assert_eq!(summary.prev_terminal(), &OLBlockCommitment::null());
    }
}
