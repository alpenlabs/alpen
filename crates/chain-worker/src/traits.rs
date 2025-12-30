//! Traits for the chain worker to interface with the underlying system.

use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_support_types::IndexerWrites;
use strata_ol_state_types::{NativeAccountState, OLState, WriteBatch};
use strata_primitives::epoch::EpochCommitment;

use crate::{OLBlockExecutionOutput, WorkerResult};

/// Context trait for a worker to interact with the database.
///
/// This trait abstracts over the database operations needed by the chain worker
/// to fetch blocks, headers, and state, as well as store execution outputs.
pub trait WorkerContext {
    // =========================================================================
    // Block access functions
    // =========================================================================

    /// Fetches a whole block by ID.
    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlock>>;

    /// Fetches block IDs at a given slot (height).
    fn fetch_blocks_at_slot(&self, slot: u64) -> WorkerResult<Vec<OLBlockId>>;

    /// Fetches a block's header by ID.
    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeader>>;

    // =========================================================================
    // Epoch/summary functions
    // =========================================================================

    /// Stores an epoch summary in the database.
    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()>;

    /// Fetches a specific epoch summary by commitment.
    fn fetch_summary(&self, epoch: &EpochCommitment) -> WorkerResult<EpochSummary>;

    /// Fetches all summaries for an epoch index.
    fn fetch_epoch_summaries(&self, epoch: u32) -> WorkerResult<Vec<EpochSummary>>;

    // =========================================================================
    // State access functions
    // =========================================================================

    /// Fetches the OL state for a given block commitment.
    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLState>>;

    /// Fetches a block's write batch by commitment.
    fn fetch_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<WriteBatch<NativeAccountState>>>;

    // =========================================================================
    // Output storage functions
    // =========================================================================

    /// Stores a block execution's output.
    ///
    /// This stores the write batch and other execution results.
    fn store_block_output(
        &self,
        commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()>;

    /// Stores auxiliary indexer data from execution.
    ///
    /// This stores inbox messages, manifests, and snark state updates.
    fn store_auxiliary_data(
        &self,
        commitment: OLBlockCommitment,
        writes: &IndexerWrites,
    ) -> WorkerResult<()>;

    /// Merges write batches up to the given epoch's state into the finalized
    /// state we accept. This means we have to load fewer write batches.
    fn merge_finalized_epoch(&self, epoch: &EpochCommitment) -> WorkerResult<()>;
}
