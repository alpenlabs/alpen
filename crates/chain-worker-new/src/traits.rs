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
/// This trait abstracts the database access layer, allowing the worker to be
/// tested with mock implementations. All methods should be blocking operations
/// as the worker runs on a dedicated thread pool.
pub trait WorkerContext: Send + Sync + 'static {
    // =========================================================================
    // Block access
    // =========================================================================

    /// Fetches a whole block by its ID.
    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlock>>;

    /// Fetches block IDs at a given slot.
    fn fetch_blocks_at_slot(&self, slot: u64) -> WorkerResult<Vec<OLBlockId>>;

    /// Fetches a block's header by its ID.
    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeader>>;

    // =========================================================================
    // State access
    // =========================================================================

    /// Fetches the OL state at a given block commitment.
    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLState>>;

    /// Fetches the write batch for a given block commitment.
    fn fetch_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<WriteBatch<NativeAccountState>>>;

    // =========================================================================
    // Output storage
    // =========================================================================

    /// Stores the block execution output (write batch, state root).
    fn store_block_output(
        &self,
        commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()>;

    /// Stores auxiliary data for indexing (inbox messages, manifests).
    fn store_auxiliary_data(
        &self,
        commitment: OLBlockCommitment,
        writes: &IndexerWrites,
    ) -> WorkerResult<()>;

    // =========================================================================
    // Epoch management
    // =========================================================================

    /// Stores an epoch summary in the database.
    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()>;

    /// Fetches a specific epoch summary by its commitment.
    fn fetch_summary(&self, epoch: &EpochCommitment) -> WorkerResult<EpochSummary>;

    /// Fetches all summaries for an epoch index.
    fn fetch_epoch_summaries(&self, epoch: u32) -> WorkerResult<Vec<EpochSummary>>;

    /// Merges write batches up to the given epoch into the finalized state.
    ///
    /// This means we have to load fewer write batches when reconstructing state.
    fn merge_finalized_epoch(&self, epoch: &EpochCommitment) -> WorkerResult<()>;
}
