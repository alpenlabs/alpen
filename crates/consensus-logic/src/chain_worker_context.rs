//! Context impl to instantiate chain worker with.

use std::sync::Arc;

use strata_chain_worker::*;
use strata_chainexec::{BlockExecutionOutput, ChangedState, CheckinExecutionOutput};
use strata_db::{
    chainstate::{StateInstanceId, WriteBatchId},
    DbError,
};
use strata_primitives::prelude::*;
use strata_state::{
    batch::EpochSummary, block::L2BlockBundle, chain_state::Chainstate, prelude::*,
    state_op::WriteBatch,
};
use strata_storage::{CheckpointDbManager, L2BlockManager, NewChainstateManager};
use tracing::*;

pub struct ChainWorkerCtx {
    l2man: Arc<L2BlockManager>,
    chsman: Arc<NewChainstateManager>,
    ckman: Arc<CheckpointDbManager>,

    /// Active state instance we build on top of for the current state.
    active_state_inst: StateInstanceId,
}

impl ChainWorkerCtx {
    pub fn new() -> Self {
        todo!()
    }
}

impl WorkerContext for ChainWorkerCtx {
    fn fetch_block(&self, blkid: &L2BlockId) -> WorkerResult<Option<L2BlockBundle>> {
        Ok(self
            .l2man
            .get_block_data_blocking(blkid)
            .map_err(conv_db_err)?)
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
        Ok(self
            .ckman
            .get_epoch_summary_blocking(*epoch)
            .map_err(conv_db_err)?
            .ok_or_else(|| WorkerError::MissingEpochSummary(*epoch))?)
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

    fn store_block_output(
        &self,
        blkid: &L2BlockId,
        output: &BlockExecutionOutput,
    ) -> WorkerResult<()> {
        let wbid = conv_blkid_to_slot_wb_id(*blkid);

        // Store the write batch from the exec output.
        self.chsman
            .put_write_batch_blocking(wbid, output.write_batch().clone())
            .map_err(conv_db_err)?;

        Ok(())
    }

    fn store_checkin_output(
        &self,
        epoch: &EpochCommitment,
        output: &CheckinExecutionOutput,
    ) -> WorkerResult<()> {
        let wbid = conv_blkid_to_epoch_terminal_wb_id(*epoch.last_blkid());

        // Store the write batch from the exec output.
        self.chsman
            .put_write_batch_blocking(wbid, output.write_batch().clone())
            .map_err(conv_db_err)?;

        Ok(())
    }

    fn fetch_block_write_batch(&self, blkid: &L2BlockId) -> WorkerResult<Option<WriteBatch>> {
        let wbid = conv_blkid_to_slot_wb_id(*blkid);
        Ok(self
            .chsman
            .get_write_batch_blocking(wbid)
            .map_err(conv_db_err)?)
    }

    fn get_finalized_toplevel_state(&self) -> WorkerResult<Arc<Chainstate>> {
        Ok(self
            .chsman
            .get_inst_toplevel_state_blocking(self.active_state_inst)
            .map_err(conv_db_err)?)
    }

    fn merge_finalized_epoch(&self, epoch: &EpochCommitment) -> WorkerResult<()> {
        let cur_tl = self.get_finalized_toplevel_state()?;

        // Check that the current state's epoch is the parent of the new epoch
        // we're merging in.
        let finalizing_epoch = self.fetch_summary(epoch)?;
        let cur_epoch_terminal = cur_tl.prev_epoch().to_block_commitment();
        if *finalizing_epoch.prev_terminal() != cur_epoch_terminal {
            // TODO make this error better
            return Err(WorkerError::Unimplemented.into());
        }

        let mut epoch_blkids = Vec::new();
        // TODO collect the blocks from this epoch back to the previous

        let mut epoch_wbids = epoch_blkids
            .into_iter()
            .map(conv_blkid_to_slot_wb_id)
            .collect::<Vec<_>>();
        epoch_wbids.push(conv_blkid_to_epoch_terminal_wb_id(*epoch.last_blkid()));

        Ok(self
            .chsman
            .merge_write_batches_blocking(self.active_state_inst, epoch_wbids)
            .map_err(conv_db_err)?)
    }
}

fn conv_db_err(e: DbError) -> WorkerError {
    // TODO fixme
    WorkerError::Unimplemented
}

fn conv_blkid_to_slot_wb_id(blkid: L2BlockId) -> WriteBatchId {
    let mut buf: Buf32 = blkid.into();
    buf.as_mut_slice()[31] = 0; // last byte to distinguish slot and epoch
    buf
}

fn conv_blkid_to_epoch_terminal_wb_id(blkid: L2BlockId) -> WriteBatchId {
    let mut buf: Buf32 = blkid.into();
    buf.as_mut_slice()[31] = 1; // last byte to distinguish slot and epoch
    buf
}
