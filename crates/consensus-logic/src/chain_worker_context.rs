//! Context impl to instantiate chain worker with.

use std::sync::Arc;

use strata_chain_worker::*;
use strata_checkpoint_types::EpochSummary;
use strata_db_types::DbError;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_support_types::IndexerWrites;
use strata_ol_state_types::{NativeAccountState, OLState, WriteBatch};
use strata_primitives::epoch::EpochCommitment;
use strata_storage::{CheckpointDbManager, OLBlockManager, OLStateManager};
use tracing::*;

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug impls"
)]
pub struct ChainWorkerCtx {
    ol_block_man: Arc<OLBlockManager>,
    ol_state_man: Arc<OLStateManager>,
    ckman: Arc<CheckpointDbManager>,
}

impl ChainWorkerCtx {
    pub fn new(
        ol_block_man: Arc<OLBlockManager>,
        ol_state_man: Arc<OLStateManager>,
        ckman: Arc<CheckpointDbManager>,
    ) -> Self {
        Self {
            ol_block_man,
            ol_state_man,
            ckman,
        }
    }
}

impl WorkerContext for ChainWorkerCtx {
    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlock>> {
        self.ol_block_man
            .get_block_data_blocking(*blkid)
            .map_err(conv_db_err)
    }

    fn fetch_blocks_at_slot(&self, slot: u64) -> WorkerResult<Vec<OLBlockId>> {
        self.ol_block_man
            .get_blocks_at_height_blocking(slot)
            .map_err(conv_db_err)
    }

    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeader>> {
        // Fetch full block and extract header
        Ok(self
            .ol_block_man
            .get_block_data_blocking(*blkid)
            .map_err(conv_db_err)?
            .map(|b| b.header().clone()))
    }

    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()> {
        self.ckman
            .insert_epoch_summary_blocking(summary)
            .map_err(conv_db_err)?;
        Ok(())
    }

    fn fetch_summary(&self, epoch: &EpochCommitment) -> WorkerResult<EpochSummary> {
        self.ckman
            .get_epoch_summary_blocking(*epoch)
            .map_err(conv_db_err)?
            .ok_or(WorkerError::MissingEpochSummary(*epoch))
    }

    fn fetch_epoch_summaries(&self, epoch: u32) -> WorkerResult<Vec<EpochSummary>> {
        let epochs = self
            .ckman
            .get_epoch_commitments_at_blocking(epoch as u64)
            .map_err(conv_db_err)?;

        let mut summaries = Vec::new();
        for epoch in epochs {
            let Some(s) = self
                .ckman
                .get_epoch_summary_blocking(epoch)
                .map_err(conv_db_err)?
            else {
                warn!(?epoch, "found epoch commitment but missing summary");
                continue;
            };

            summaries.push(s);
        }

        Ok(summaries)
    }

    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLState>> {
        self.ol_state_man
            .get_toplevel_ol_state_blocking(commitment)
            .map_err(conv_db_err)
            .map(|opt| opt.map(|arc| (*arc).clone()))
    }

    fn fetch_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<WriteBatch<NativeAccountState>>> {
        self.ol_state_man
            .get_write_batch_blocking(commitment)
            .map_err(conv_db_err)
    }

    fn store_block_output(
        &self,
        commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        // Store the write batch
        self.ol_state_man
            .put_write_batch_blocking(commitment, output.write_batch().clone())
            .map_err(conv_db_err)?;

        Ok(())
    }

    fn store_auxiliary_data(
        &self,
        _commitment: OLBlockCommitment,
        _writes: &IndexerWrites,
    ) -> WorkerResult<()> {
        // TODO: Store inbox messages via InboxMessageDatabase
        // For now, this is a no-op since we don't have the inbox message manager
        // wired up yet.
        Ok(())
    }

    fn merge_finalized_epoch(&self, _epoch: &EpochCommitment) -> WorkerResult<()> {
        // TODO: Implement epoch finalization logic
        // This will merge write batches into the finalized state
        Ok(())
    }
}

fn conv_db_err(e: DbError) -> WorkerError {
    WorkerError::Database(e.to_string())
}

// ============================================================================
// Legacy helper functions for L2BlockId -> WriteBatchId conversion
// Used by dbtool and other utilities for the old chainstate database
// ============================================================================

use strata_db_types::chainstate::WriteBatchId;
use strata_ol_chain_types::L2BlockId;
use strata_primitives::buf::Buf32;

/// Converts an L2 block ID to a slot write batch ID.
///
/// This is used for the legacy chainstate database where write batches
/// are indexed by a modified form of the block ID.
pub fn conv_blkid_to_slot_wb_id(blkid: L2BlockId) -> WriteBatchId {
    let mut buf: Buf32 = blkid.into();
    buf.as_mut_slice()[31] = 0; // last byte to distinguish slot and epoch
    buf
}

/// Converts an L2 block ID to an epoch terminal write batch ID.
///
/// This is used for the legacy chainstate database where epoch terminal
/// write batches are indexed by a modified form of the block ID.
pub fn conv_blkid_to_epoch_terminal_wb_id(blkid: L2BlockId) -> WriteBatchId {
    let mut buf: Buf32 = blkid.into();
    buf.as_mut_slice()[31] = 1; // last byte to distinguish slot and epoch
    buf
}
