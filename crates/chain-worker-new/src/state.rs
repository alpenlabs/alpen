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

use strata_acct_types::AccountId;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::OLBlockCommitment;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_da::{DaScheme, OLDaSchemeV1};
use strata_ol_state_support_types::{IndexerState, IndexerWrites, WriteTrackingState};
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
use strata_ol_stf::{
    BasicExecContext, BlockInfo, EpochInitialContext, ExecOutputBuffer, process_block_manifests,
    process_epoch_initial, verify_block,
};
use strata_primitives::{epoch::EpochCommitment, l1::L1BlockCommitment};
use strata_service::ServiceState;
use tracing::*;

use crate::{
    ApplyDAPayload, ChainWorkerContextImpl,
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
    #[instrument(skip(self), fields(slot = block_commitment.slot(), %block_commitment))]
    pub(crate) fn try_exec_block(
        &mut self,
        block_commitment: &OLBlockCommitment,
    ) -> WorkerResult<()> {
        self.check_initialized()?;

        debug!("trying to execute block");

        // Fetch block and parent context
        let (block, parent_header, parent_commitment) =
            self.fetch_block_with_parent(block_commitment)?;

        // Execute STF and get output and new state
        let (output, new_state) =
            self.execute_stf(&block, parent_header.as_ref(), parent_commitment)?;

        // Persist results (including the full state)
        self.persist_execution_output(&block, *block_commitment, &output, new_state)?;

        // Handle epoch terminal if needed
        debug!(
            is_terminal = block.header().is_terminal(),
            "checking if block is terminal"
        );
        if block.header().is_terminal() {
            self.handle_complete_epoch(&block, &output)?;
            // Send the epoch commitment to receiver
            // TODO: it seems to be done for each block at the moment. Ideally we would do it just
            // here.
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
    ) -> WorkerResult<(OLBlockExecutionOutput, OLState)> {
        // Fetch parent state
        let parent_state = self
            .ctx
            .fetch_ol_state(parent_commitment)?
            .ok_or(WorkerError::MissingPreState(parent_commitment))?;

        // Execute and extract outputs
        let (write_batch, indexer_writes) =
            Self::run_stf_verification(&parent_state, block, parent_header)?;

        // Apply write batch to parent state to get new state
        let mut new_state = parent_state;
        new_state
            .apply_write_batch(write_batch.clone())
            .map_err(|e| WorkerError::Unexpected(format!("Failed to apply write batch: {}", e)))?;

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
        parent_state: &OLState,
        block: &OLBlock,
        parent_header: Option<&OLBlockHeader>,
    ) -> WorkerResult<(WriteBatch<OLAccountState>, IndexerWrites)> {
        // Build the state stack: IndexerState<WriteTrackingState<&OLState>>
        let tracking_state = WriteTrackingState::new_from_state(parent_state);
        let mut indexer_state = IndexerState::new(tracking_state);

        verify_block(
            &mut indexer_state,
            block.header(),
            parent_header,
            block.body(),
        )?;

        // Extract outputs
        let (tracking_state, indexer_writes) = indexer_state.into_parts();
        let write_batch = tracking_state.into_batch();

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
    #[instrument(skip_all, fields(epoch = block.header().epoch(), slot = block.header().slot()))]
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
        let prev_ep_num = completed_epoch.saturating_sub(1);
        let prev_terminal = if completed_epoch == 0 {
            OLBlockCommitment::null()
        } else {
            *self
                .ctx
                .fetch_canonical_epoch_summary_at(prev_ep_num)?
                .ok_or(WorkerError::MissingEpochSummaryAt(prev_ep_num))?
                .terminal()
        };

        // Get L1 info from the write batch (epochal state has latest L1 after manifest sealing)
        let epochal = last_block_output.write_batch().epochal();
        let new_tip_height = epochal.last_l1_height();
        let new_tip_blkid = epochal.last_l1_blkid();
        let new_l1_block = L1BlockCommitment::new(new_tip_height, *new_tip_blkid);

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
        debug!(slot = tip.slot(), %tip, "updating safe tip");
        self.state.cur_tip = tip;
        Ok(())
    }

    /// Finalizes an epoch, merging write batches into finalized state.
    pub(crate) fn finalize_epoch(&mut self, epoch: EpochCommitment) -> WorkerResult<()> {
        info!(epoch_num = epoch.epoch(), %epoch, "finalizing epoch");
        self.state.last_finalized_epoch = Some(epoch);
        Ok(())
    }

    #[instrument(skip(self, da_payload), fields(epoch_num = da_payload.epoch.epoch(), last_slot = da_payload.epoch.last_slot))]
    pub(crate) fn apply_da(&self, da_payload: ApplyDAPayload) -> WorkerResult<()> {
        let ApplyDAPayload {
            da_payload: da,
            epoch,
            manifests,
            terminal_header_complement,
        } = da_payload;

        info!("applying DA for epoch");

        // Fetch previous state
        let (state, prev_terminal) = fetch_prev_state(&self.ctx, &epoch)?;

        // Extract new account IDs before the payload is consumed.
        let new_account_ids: Vec<AccountId> = da
            .state_diff
            .ledger
            .new_accounts
            .entries()
            .iter()
            .map(|e| e.account_id)
            .collect();

        debug!(
            new_accounts = new_account_ids.len(),
            "extracted new account IDs from DA payload"
        );

        // Prepare data for processing epoch initial
        let epctx = EpochInitialContext::new(epoch.epoch(), epoch.to_block_commitment());

        // Wrap state to collect index data
        let mut indexer_state = IndexerState::new(state);

        debug!("processing epoch initial");
        process_epoch_initial(&mut indexer_state, &epctx)?;

        debug!("applying state diff");
        OLDaSchemeV1::apply_to_state(da, &mut indexer_state)
            .map_err(|e| WorkerError::DaApplication(epoch, e))?;

        // Prepare data for processing manifests
        let outbuf = ExecOutputBuffer::new_empty();
        let timestamp = terminal_header_complement.timestamp();
        let blkinfo = BlockInfo::new(timestamp, epoch.last_slot, epoch.epoch());

        debug!("processing ASM manifests");
        let exctx = BasicExecContext::new(blkinfo, &outbuf);
        process_block_manifests(&mut indexer_state, &manifests, &exctx)?;

        let (state, indexer_writes) = indexer_state.into_parts();
        let terminal_commitment = epoch.to_block_commitment();

        // Extract summary data before state is moved into storage.
        let new_l1 = L1BlockCommitment::new(state.last_l1_height(), *state.last_l1_blkid());
        let epoch_final_state_root = state
            .compute_state_root()
            .map_err(|_| WorkerError::StateRootComputation)?;

        debug!("persisting DA output to storage");
        self.ctx.store_da_output(
            terminal_commitment,
            epoch.epoch(),
            state,
            &new_account_ids,
            &indexer_writes,
        )?;

        let summary = EpochSummary::new(
            epoch.epoch(),
            terminal_commitment,
            prev_terminal,
            new_l1,
            epoch_final_state_root,
        );
        debug!(?summary, "storing epoch summary from DA");
        self.ctx.store_summary(summary)?;

        info!("DA applied successfully");

        Ok(())
    }
}

/// Fetch state corresponding to previous epoch.
#[instrument(skip(ctx), fields(epoch_num = epoch.epoch()))]
fn fetch_prev_state(
    ctx: &ChainWorkerContextImpl,
    epoch: &EpochCommitment,
) -> WorkerResult<(OLState, OLBlockCommitment)> {
    let prev_summary = ctx
        .fetch_canonical_epoch_summary_at(epoch.epoch().saturating_sub(1))?
        .ok_or(WorkerError::MissingEpochSummary(*epoch))?;
    let prev_terminal = *prev_summary.terminal();
    debug!(?prev_terminal, "fetching previous terminal state");
    let st = ctx
        .fetch_ol_state(prev_terminal)?
        .ok_or(WorkerError::MissingPreState(prev_terminal))?;
    Ok((st, prev_terminal))
}

impl ServiceState for ChainWorkerServiceState {
    fn name(&self) -> &str {
        "chain_worker_new"
    }
}
