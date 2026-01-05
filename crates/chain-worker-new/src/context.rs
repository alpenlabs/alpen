//! Concrete implementation of the [`ChainWorkerContext`] trait.
//!
//! This module provides [`ChainWorkerContextImpl`], a production implementation
//! of the worker context that uses the storage layer managers for database access.

use std::sync::Arc;

use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_support_types::IndexerWrites;
use strata_ol_state_types::{NativeAccountState, OLState, WriteBatch};
use strata_primitives::epoch::EpochCommitment;
use strata_storage::{CheckpointDbManager, OLBlockManager, OLStateManager};
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
    checkpoint_mgr: Arc<CheckpointDbManager>,
}

impl ChainWorkerContextImpl {
    /// Creates a new context with the given storage managers.
    pub fn new(
        ol_block_mgr: Arc<OLBlockManager>,
        ol_state_mgr: Arc<OLStateManager>,
        checkpoint_mgr: Arc<CheckpointDbManager>,
    ) -> Self {
        Self {
            ol_block_mgr,
            ol_state_mgr,
            checkpoint_mgr,
        }
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

    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLState>> {
        let state_opt = self
            .ol_state_mgr
            .get_toplevel_ol_state_blocking(commitment)?;
        Ok(state_opt.map(|arc| (*arc).clone()))
    }

    fn fetch_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<WriteBatch<NativeAccountState>>> {
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

    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()> {
        self.checkpoint_mgr.insert_epoch_summary_blocking(summary)?;
        Ok(())
    }

    fn fetch_summary(&self, epoch: &EpochCommitment) -> WorkerResult<EpochSummary> {
        self.checkpoint_mgr
            .get_epoch_summary_blocking(*epoch)?
            .ok_or(WorkerError::MissingEpochSummary(*epoch))
    }

    fn fetch_epoch_summaries(&self, epoch: u32) -> WorkerResult<Vec<EpochSummary>> {
        // Get all epoch commitments for this epoch index
        let epoch_commitments = self
            .checkpoint_mgr
            .get_epoch_commitments_at_blocking(epoch as u64)?;

        // Fetch the summary for each commitment
        let mut summaries = Vec::with_capacity(epoch_commitments.len());
        for commitment in epoch_commitments {
            if let Some(summary) = self.checkpoint_mgr.get_epoch_summary_blocking(commitment)? {
                summaries.push(summary);
            }
        }

        Ok(summaries)
    }

    fn merge_finalized_epoch(&self, epoch: &EpochCommitment) -> WorkerResult<()> {
        // Get the epoch summary to find the block range
        let summary = self.fetch_summary(epoch)?;

        // Get the current finalized state (or start from prev_terminal's state)
        let prev_terminal = *summary.prev_terminal();
        let mut current_state = if prev_terminal.is_null() {
            // Genesis case - get genesis state
            self.fetch_ol_state(OLBlockCommitment::null())?
                .ok_or(WorkerError::MissingPreState(OLBlockCommitment::null()))?
        } else {
            self.fetch_ol_state(prev_terminal)?
                .ok_or(WorkerError::MissingPreState(prev_terminal))?
        };

        // Walk through all blocks from prev_terminal to terminal and apply write batches
        let terminal = *summary.terminal();
        let mut current_slot = prev_terminal.slot();

        // Process blocks slot by slot until we reach the terminal
        while current_slot < terminal.slot() {
            current_slot += 1;

            // Get blocks at this slot
            let block_ids = self.fetch_blocks_at_slot(current_slot)?;

            // For each block, check if it's in our chain and apply its write batch
            for blkid in block_ids {
                let commitment = OLBlockCommitment::new(current_slot, blkid);

                // Try to get the write batch - if it exists, this block was executed
                if let Some(wb) = self.fetch_write_batch(commitment)? {
                    // Apply the write batch to advance the state
                    current_state.apply_write_batch(wb).map_err(|e| {
                        WorkerError::Unexpected(format!("failed to apply batch: {e}"))
                    })?;
                }
            }
        }

        // Store the final merged state at the terminal commitment
        self.ol_state_mgr
            .put_toplevel_ol_state_blocking(terminal, current_state)?;

        Ok(())
    }
}
