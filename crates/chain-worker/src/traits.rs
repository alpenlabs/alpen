//! Traits for the chain worker to interface with the underlying system.

use strata_chainexec::BlockExecutionOutput;
use strata_primitives::{batch::EpochSummary, prelude::*};
use strata_state::{block::L2BlockBundle, header::L2BlockHeader, state_op::WriteBatch};

use crate::WorkerResult;

/// Context trait for a worker to interact with the database.
pub trait WorkerContext {
    /// Fetches a whole block bundle.
    fn fetch_block(&self, blkid: &L2BlockId) -> WorkerResult<Option<L2BlockBundle>>;

    /// Fetches a block's header.
    fn fetch_header(&self, blkid: &L2BlockId) -> WorkerResult<Option<L2BlockHeader>>;

    /// Fetches a block's write batch.
    fn fetch_block_write_batch(&self, blkid: &L2BlockId) -> WorkerResult<Option<WriteBatch>>;

    /// Stores a block execution's output.  This MAY be broken up into multiple
    /// separate pieces.
    fn store_block_output(
        &self,
        blkid: &L2BlockId,
        output: BlockExecutionOutput,
    ) -> WorkerResult<()>;

    /// Fetches all summaries for an epoch index.
    fn fetch_epoch_summaries(&self, epoch: u32) -> WorkerResult<Vec<EpochSummary>>;

    /// Fetches a specific epoch summary.
    fn fetch_summary(&self, epoch: &EpochCommitment) -> WorkerResult<EpochSummary>;

    /// Stores an epoch summary in the database.
    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()>;
}
