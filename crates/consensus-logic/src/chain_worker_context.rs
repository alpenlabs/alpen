//! Context impl to instantiate chain worker with.

use std::sync::Arc;

use strata_chain_worker::*;
use strata_chainexec::{BlockExecutionOutput, CheckinExecutionOutput};
use strata_db::{
    chainstate::{StateInstanceId, WriteBatchId},
    DbError,
};
use strata_primitives::prelude::*;
use strata_state::{
    batch::EpochSummary, block::L2BlockBundle, chain_state::Chainstate, prelude::*,
    state_op::WriteBatch,
};
use strata_storage::{ChainstateManager, CheckpointDbManager, L2BlockManager};
use tracing::*;

#[expect(missing_debug_implementations)]
pub struct ChainWorkerCtx {
    l2man: Arc<L2BlockManager>,
    chsman: Arc<ChainstateManager>,
    ckman: Arc<CheckpointDbManager>,

    /// Active state instance we build on top of for the current state.
    active_state_inst: StateInstanceId,
}

impl ChainWorkerCtx {
    pub fn new(
        l2man: Arc<L2BlockManager>,
        chsman: Arc<ChainstateManager>,
        ckman: Arc<CheckpointDbManager>,
        active_state_inst: StateInstanceId,
    ) -> Self {
        Self {
            l2man,
            chsman,
            ckman,
            active_state_inst,
        }
    }
}

impl WorkerContext for ChainWorkerCtx {
    fn fetch_block(&self, blkid: &L2BlockId) -> WorkerResult<Option<L2BlockBundle>> {
        self.l2man
            .get_block_data_blocking(blkid)
            .map_err(conv_db_err)
    }

    fn fetch_block_ids(&self, height: u64) -> WorkerResult<Vec<L2BlockId>> {
        self.l2man
            .get_blocks_at_height_blocking(height)
            .map_err(conv_db_err)
    }

    fn fetch_header(&self, blkid: &L2BlockId) -> WorkerResult<Option<L2BlockHeader>> {
        // FIXME make this only fetch the header
        Ok(self
            .l2man
            .get_block_data_blocking(blkid)
            .map_err(conv_db_err)?
            .map(|b| b.header().header().clone()))
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

    // Store the write batch from the exec output.
    fn store_block_output(
        &self,
        blkid: &L2BlockId,
        output: &BlockExecutionOutput,
    ) -> WorkerResult<()> {
        self.chsman
            .put_slot_write_batch_blocking(*blkid, output.write_batch().clone())
            .map_err(conv_db_err)?;

        Ok(())
    }

    // Store the write batch from the exec output.
    fn store_checkin_output(
        &self,
        epoch: &EpochCommitment,
        output: &CheckinExecutionOutput,
    ) -> WorkerResult<()> {
        self.chsman
            .put_epoch_write_batch_blocking(*epoch.last_blkid(), output.write_batch().clone())
            .map_err(conv_db_err)?;

        Ok(())
    }

    fn fetch_block_write_batch(&self, blkid: &L2BlockId) -> WorkerResult<Option<WriteBatch>> {
        self.chsman
            .get_slot_write_batch_blocking(*blkid)
            .map_err(conv_db_err)
    }

    fn get_finalized_toplevel_state(&self) -> WorkerResult<Arc<Chainstate>> {
        self.chsman
            .get_inst_toplevel_state_blocking(self.active_state_inst)
            .map_err(conv_db_err)
    }

    fn merge_finalized_epoch(&self, epoch: &EpochCommitment) -> WorkerResult<()> {
        let cur_tl = self.get_finalized_toplevel_state()?;

        // Check that the current state's epoch is the parent of the new epoch
        // we're merging in.
        let finalizing_epoch = self.fetch_summary(epoch)?;
        let cur_epoch_terminal = cur_tl.prev_epoch().to_block_commitment();
        if *finalizing_epoch.prev_terminal() != cur_epoch_terminal {
            // TODO make this error better
            return Err(WorkerError::Unimplemented);
        }

        let epoch_blkids = Vec::new();
        // TODO collect the blocks from this epoch back to the previous

        let mut epoch_wbids = epoch_blkids
            .into_iter()
            .map(conv_blkid_to_slot_wb_id)
            .collect::<Vec<_>>();
        epoch_wbids.push(conv_blkid_to_epoch_terminal_wb_id(*epoch.last_blkid()));

        self.chsman
            .merge_write_batches_blocking(self.active_state_inst, epoch_wbids)
            .map_err(conv_db_err)
    }
}

fn conv_db_err(_e: DbError) -> WorkerError {
    // TODO fixme
    WorkerError::Unimplemented
}

// FIXME: fix duplicate code
pub fn conv_blkid_to_slot_wb_id(blkid: L2BlockId) -> WriteBatchId {
    let mut buf: Buf32 = blkid.into();
    buf.as_mut_slice()[31] = 0; // last byte to distinguish slot and epoch
    buf
}

pub fn conv_blkid_to_epoch_terminal_wb_id(blkid: L2BlockId) -> WriteBatchId {
    let mut buf: Buf32 = blkid.into();
    buf.as_mut_slice()[31] = 1; // last byte to distinguish slot and epoch
    buf
}
