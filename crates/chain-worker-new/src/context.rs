//! Concrete implementation of the [`ChainWorkerContext`] trait.
//!
//! This module provides [`ChainWorkerContextImpl`], a production implementation
//! of the worker context that uses the storage layer managers for database access.

use std::sync::Arc;

use strata_checkpoint_types::EpochSummary;
use strata_db_types::{DbResult, types::AccountExtraDataEntry};
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_node_context::NodeContext;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_support_types::IndexerWrites;
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
use strata_params::Params;
use strata_primitives::epoch::EpochCommitment;
use strata_status::StatusChannel;
use strata_storage::{AccountManager, OLBlockManager, OLCheckpointManager, OLStateManager};
use tokio::{runtime::Handle, sync::watch};
use tracing::warn;

use crate::{
    errors::{WorkerError, WorkerResult},
    output::OLBlockExecutionOutput,
    traits::ChainWorkerContext,
};

/// Concrete implementation of [`ChainWorkerContext`] using storage managers.
///
/// This implementation wraps the high-level storage managers to provide
/// database access for the chain worker. All operations are blocking as
/// the worker runs on a dedicated thread pool.
#[expect(
    missing_debug_implementations,
    reason = "Storage managers don't implement Debug"
)]
pub struct ChainWorkerContextImpl {
    /// Manager for OL block data (headers + bodies).
    ol_block_mgr: Arc<OLBlockManager>,

    /// Manager for OL state snapshots and write batches.
    ol_state_mgr: Arc<OLStateManager>,

    /// Manager for checkpoint and epoch summary data.
    ol_checkpoint_mgr: Arc<OLCheckpointManager>,

    /// Manager for per-account creation epoch tracking.
    account_mgr: Arc<AccountManager>,

    /// Status channel to send/receive messages.
    status_channel: Arc<StatusChannel>,

    /// Channel for emitting epoch summary events.
    epoch_summary_tx: watch::Sender<Option<EpochCommitment>>,

    /// Rollup params
    params: Arc<Params>,

    /// Runtime handle
    handle: Handle,
}

impl ChainWorkerContextImpl {
    /// Creates a new context with the given storage managers.
    pub fn from_node_context(nodectx: &NodeContext) -> Self {
        let (epoch_summary_tx, _) = watch::channel(None);
        Self {
            ol_block_mgr: nodectx.storage().ol_block().clone(),
            ol_state_mgr: nodectx.storage().ol_state().clone(),
            ol_checkpoint_mgr: nodectx.storage().ol_checkpoint().clone(),
            account_mgr: nodectx.storage().account().clone(),
            status_channel: nodectx.status_channel().clone(),
            epoch_summary_tx,
            params: nodectx.params().clone(),
            handle: nodectx.executor().handle().clone(),
        }
    }

    pub fn epoch_summary_sender(&self) -> watch::Sender<Option<EpochCommitment>> {
        self.epoch_summary_tx.clone()
    }

    pub fn status_channel(&self) -> &StatusChannel {
        &self.status_channel
    }

    pub fn params(&self) -> &Params {
        &self.params
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }
}

impl ChainWorkerContext for ChainWorkerContextImpl {
    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlock>> {
        Ok(self.ol_block_mgr.get_block_data_blocking(*blkid)?)
    }

    fn fetch_blocks_at_slot(&self, slot: u64) -> WorkerResult<Vec<OLBlockId>> {
        Ok(self.ol_block_mgr.get_blocks_at_height_blocking(slot)?)
    }

    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeader>> {
        // Fetch the full block and extract just the header
        let block_opt = self.ol_block_mgr.get_block_data_blocking(*blkid)?;
        Ok(block_opt.map(|block| block.header().clone()))
    }

    fn fetch_chain_tip(&self) -> WorkerResult<Option<OLBlockCommitment>> {
        // Get the highest slot with a block
        let tip_slot = self.ol_block_mgr.get_tip_slot_blocking()?;

        // Slot 0 with no blocks means no chain yet
        if tip_slot == 0 {
            let blocks = self.fetch_blocks_at_slot(0)?;
            if blocks.is_empty() {
                return Ok(None);
            }
        }

        // Get blocks at the tip slot
        let block_ids = self.fetch_blocks_at_slot(tip_slot)?;

        // Return the first block at the tip slot
        // If there are multiple (forks), we just pick one - the caller can
        // use fork choice logic if needed
        let blkid = match block_ids.first() {
            Some(id) => *id,
            None => return Ok(None),
        };

        Ok(Some(OLBlockCommitment::new(tip_slot, blkid)))
    }

    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLState>> {
        let state_opt = self
            .ol_state_mgr
            .get_toplevel_ol_state_blocking(commitment)?;
        Ok(state_opt.map(|arc| (*arc).clone()))
    }

    fn fetch_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<WriteBatch<OLAccountState>>> {
        Ok(self.ol_state_mgr.get_write_batch_blocking(commitment)?)
    }

    fn store_block_output(
        &self,
        commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        // Store the write batch
        self.ol_state_mgr
            .put_write_batch_blocking(commitment, output.write_batch().clone())?;

        // Record creation epoch for newly created accounts.
        let wb = output.write_batch();
        let epoch = wb.epochal().cur_epoch();
        wb.ledger().iter_new_accounts().try_for_each(|(_, id)| {
            self.account_mgr
                .insert_account_creation_epoch_blocking(*id, epoch)
            // TODO: might need to account for extra data as well
        })?;

        // Write account extra data
        let mut snark_updates_iter = output.indexer_writes().snark_state_updates().iter();
        snark_updates_iter.try_for_each(|update| -> DbResult<()> {
            let acct_id = update.account_id();
            if let Some(extra_data) = update.extra_data() {
                // NOTE: this is expected to be updated for given epoch at every block that contains
                // extra data for this account
                let key = (acct_id, epoch);
                let entry = AccountExtraDataEntry::new(extra_data.to_vec(), commitment);
                self.account_mgr
                    .insert_account_extra_data_blocking(key, entry)?
            }
            Ok(())
        })?;

        Ok(())
    }

    fn store_auxiliary_data(
        &self,
        _commitment: OLBlockCommitment,
        writes: &IndexerWrites,
    ) -> WorkerResult<()> {
        // TODO: IndexerWrites needs Borsh serialization before it can be stored.
        // This requires adding BorshSerialize/BorshDeserialize to IndexerWrites
        // and all its sub-types (InboxMessageWrite, ManifestWrite, SnarkAcctStateUpdate,
        // etc.), which cascades to types in other crates that may not have Borsh.
        //
        // For now, we log a warning if there are writes to store but skip storage.
        // This should be addressed in a follow-up PR that adds serialization support.
        if !writes.is_empty() {
            warn!(
                inbox_messages = writes.inbox_messages().len(),
                manifests = writes.manifests().len(),
                snark_updates = writes.snark_state_updates().len(),
                "skipping auxiliary data storage - IndexerWrites serialization not implemented"
            );
        }
        Ok(())
    }

    fn store_toplevel_state(
        &self,
        commitment: OLBlockCommitment,
        state: OLState,
    ) -> WorkerResult<()> {
        self.ol_state_mgr
            .put_toplevel_ol_state_blocking(commitment, state)?;
        Ok(())
    }

    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()> {
        let commitment = summary.get_epoch_commitment();
        self.ol_checkpoint_mgr
            .insert_epoch_summary_blocking(summary)?;
        let _ = self.epoch_summary_tx.send(Some(commitment));
        Ok(())
    }

    fn fetch_summary(&self, epoch: &EpochCommitment) -> WorkerResult<EpochSummary> {
        self.ol_checkpoint_mgr
            .get_epoch_summary_blocking(*epoch)?
            .ok_or(WorkerError::MissingEpochSummary(*epoch))
    }

    fn fetch_epoch_summaries(&self, epoch: u32) -> WorkerResult<Vec<EpochSummary>> {
        // Get all epoch commitments for this epoch index
        let epoch_commitments = self
            .ol_checkpoint_mgr
            .get_epoch_commitments_at_blocking(epoch as u64)?;

        // Fetch the summary for each commitment
        let mut summaries = Vec::with_capacity(epoch_commitments.len());
        for commitment in epoch_commitments {
            if let Some(summary) = self
                .ol_checkpoint_mgr
                .get_epoch_summary_blocking(commitment)?
            {
                summaries.push(summary);
            }
        }

        Ok(summaries)
    }

    fn merge_finalized_epoch(&self, epoch: &EpochCommitment) -> WorkerResult<()> {
        let summary = self.fetch_summary(epoch)?;
        let terminal = *summary.terminal();
        let prev_terminal = *summary.prev_terminal();

        // Collect canonical chain by walking backwards from terminal via parent pointers.
        // This ensures we only apply write batches for blocks in the canonical chain,
        // not fork blocks that may also have write batches stored.
        let mut chain: Vec<OLBlockCommitment> = Vec::new();
        let mut current = terminal;

        while current != prev_terminal && !current.is_null() {
            chain.push(current);
            // Get header to find parent
            let header = self
                .fetch_header(current.blkid())?
                .ok_or(WorkerError::MissingOLBlock(*current.blkid()))?;
            let parent_blkid = header.parent_blkid();
            if parent_blkid.is_null() {
                break;
            }
            current = OLBlockCommitment::new(current.slot().saturating_sub(1), *parent_blkid);
        }

        // Reverse to get forward order (excluding prev_terminal which is already finalized)
        chain.reverse();

        // Get base state from prev_terminal (or genesis)
        let mut current_state = if prev_terminal.is_null() {
            self.fetch_ol_state(OLBlockCommitment::null())?
                .ok_or(WorkerError::MissingPreState(OLBlockCommitment::null()))?
        } else {
            self.fetch_ol_state(prev_terminal)?
                .ok_or(WorkerError::MissingPreState(prev_terminal))?
        };

        // Apply write batches in canonical order.
        // Every block in the canonical chain must have a write batch - a missing one
        // indicates data corruption or a bug, so we error out rather than skip.
        for commitment in chain {
            let wb = self
                .fetch_write_batch(commitment)?
                .ok_or(WorkerError::MissingWriteBatch(commitment))?;
            current_state
                .apply_write_batch(wb)
                .map_err(|e| WorkerError::Unexpected(format!("failed to apply batch: {e}")))?;
        }

        // Store the final merged state at the terminal commitment
        self.ol_state_mgr
            .put_toplevel_ol_state_blocking(terminal, current_state)?;

        Ok(())
    }
}
