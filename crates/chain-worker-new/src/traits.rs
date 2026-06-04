//! Traits for the chain worker to interface with the underlying system.

use strata_asm_common::AsmManifest;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Epoch, OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
use strata_primitives::epoch::EpochCommitment;

use crate::{OLBlockExecutionOutput, WorkerResult};

/// Context trait for a worker to interact with the database.
///
/// This trait abstracts the database access layer, allowing the worker to be
/// tested with mock implementations. All methods should be blocking operations
/// as the worker runs on a dedicated thread pool.
pub trait ChainWorkerContext: Send + Sync + 'static {
    // =========================================================================
    // Block access
    // =========================================================================

    /// Fetches a whole block by its ID.
    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlock>>;

    /// Fetches block IDs at a given slot.
    fn fetch_blocks_at_slot(&self, slot: u64) -> WorkerResult<Vec<OLBlockId>>;

    /// Fetches a block's header by its ID.
    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeader>>;

    /// Fetches the current chain tip from the database.
    ///
    /// Returns the highest slot block that has been stored. If there are multiple
    /// blocks at the tip slot (forks), returns one of them.
    /// Returns `None` if no blocks have been stored yet.
    fn fetch_chain_tip(&self) -> WorkerResult<Option<OLBlockCommitment>>;

    // =========================================================================
    // State access
    // =========================================================================

    /// Fetches the OL state at a given block commitment.
    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLState>>;

    /// Fetches the write batch for a given block commitment.
    fn fetch_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<WriteBatch<OLAccountState>>>;

    // =========================================================================
    // Output storage
    // =========================================================================

    /// Stores the block execution output: write batch, indexing data
    /// (creation epochs, per-account update records, inbox writes).
    fn store_block_output(
        &self,
        block: &OLBlock,
        commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()>;

    /// Stores the full toplevel state for a block.
    fn store_toplevel_state(
        &self,
        commitment: OLBlockCommitment,
        state: OLState,
    ) -> WorkerResult<()>;

    // =========================================================================
    // Epoch management
    // =========================================================================

    /// Stores an epoch summary in the database.
    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()>;

    /// Fetches a specific epoch summary by its commitment.
    fn fetch_summary(&self, epoch: &EpochCommitment) -> WorkerResult<EpochSummary>;

    /// Fetches canonical epoch summary for an epoch index.
    fn fetch_canonical_epoch_summary_at(&self, epoch: Epoch) -> WorkerResult<Option<EpochSummary>>;

    /// Merges write batches of an epoch and stores the result.
    fn merge_epoch_data(&self, epoch: &EpochCommitment) -> WorkerResult<()>;

    /// Seeds the DB-side L1 block refs MMR mirror with sentinel leaves matching
    /// the in-state MMR's genesis prefill. Called once at chain worker init.
    /// Idempotent across restarts.
    fn prefill_l1_block_refs_mmr(&self) -> WorkerResult<()>;

    /// Fetches the checkpoint payload observed on L1 for the given epoch.
    ///
    /// This is the payload extracted from a buried L1 checkpoint, used by
    /// checkpoint sync to reconstruct epoch state.
    fn fetch_checkpoint_payload(
        &self,
        epoch: &EpochCommitment,
    ) -> WorkerResult<Option<CheckpointPayload>>;

    /// Fetches ASM manifests for the inclusive L1 height range `[from, to]`.
    ///
    /// Used to replay manifest processing during DA-based epoch reconstruction.
    fn fetch_l1_manifests(&self, from: u32, to: u32) -> WorkerResult<Vec<AsmManifest>>;

    /// Applies epoch-granular state index writes for a reconstructed epoch,
    /// derived from the epoch's execution `output`.
    fn apply_epoch_indexing(
        &self,
        epoch: &EpochCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()>;
}
