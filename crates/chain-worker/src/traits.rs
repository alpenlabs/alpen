//! Traits for the chain worker to interface with the underlying system.

use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{L1BlockCommitment, OLBlockCommitment, OLBlockId};
use strata_ol_block_ancestry_provider::EpochBlockAncestry;
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_types_v1::WriteBatch;
use strata_primitives::epoch::EpochCommitment;

use crate::{OLBlockExecutionOutput, WorkerResult};

/// Context trait for a worker to interact with the database.
///
/// This trait abstracts the database access layer, allowing the worker to be
/// tested with mock implementations. All methods should be blocking operations
/// as the worker runs on a dedicated thread pool.
pub trait ChainWorkerContext: Send + Sync + 'static {
    /// Returns the runtime parameters used for OL STF execution.
    fn runtime_params(&self) -> OLRuntimeParams;

    /// Returns the L1 block OL genesis anchors to.
    ///
    /// Genesis takes it as the state's last L1 block without processing a
    /// manifest for it, so it starts the genesis epoch's L1 range.
    fn genesis_l1_block(&self) -> L1BlockCommitment;

    // =========================================================================
    // Block access
    // =========================================================================

    /// Fetches a whole block by its ID.
    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockV1>>;

    /// Fetches block IDs at a given slot.
    fn fetch_blocks_at_slot(&self, slot: u64) -> WorkerResult<Vec<OLBlockId>>;

    /// Fetches a block's header by its ID.
    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeaderV1>>;

    /// Collects blocks through `start` in execution order and resolves the previous terminal.
    ///
    /// Follows `start`'s branch; missing or inconsistent ancestry returns an error.
    fn collect_epoch_block_commitments(
        &self,
        start: &OLBlockHeaderV1,
    ) -> WorkerResult<EpochBlockAncestry>;

    /// Fetches the current chain tip from the database.
    ///
    /// Returns the canonical OL tip, or `None` if no canonical block has been stored yet.
    fn fetch_chain_tip(&self) -> WorkerResult<Option<OLBlockCommitment>>;

    // =========================================================================
    // State access
    // =========================================================================

    /// Fetches the OL state at a given block commitment.
    fn fetch_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<OLStateContainer>>;

    /// Fetches the write batch for a given block commitment.
    fn fetch_write_batch(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<WriteBatch>>;

    // =========================================================================
    // Output storage
    // =========================================================================

    /// Stores the block execution output: write batch, indexing data
    /// (creation epochs, per-account update records, inbox writes).
    fn store_block_output(
        &self,
        block: &OLBlockV1,
        commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()>;

    /// Stores the full toplevel state for a block.
    fn store_toplevel_state(
        &self,
        commitment: OLBlockCommitment,
        state: OLStateContainer,
    ) -> WorkerResult<()>;

    /// Stores an unsigned checkpoint terminal [`OLBlockHeaderV1`].
    fn store_terminal_header(&self, id: OLBlockId, header: OLBlockHeaderV1) -> WorkerResult<()>;

    // =========================================================================
    // Epoch management
    // =========================================================================

    /// Stores the summary after the epoch's state and indexing writes succeed.
    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()>;

    /// Deletes the summary for exactly this epoch commitment, returning whether it existed.
    fn delete_summary(&self, epoch: EpochCommitment) -> WorkerResult<bool>;

    /// Fetches the summary for exactly this epoch commitment.
    fn fetch_summary(&self, epoch: EpochCommitment) -> WorkerResult<Option<EpochSummary>>;

    /// Merges an epoch's write batches and stores its terminal state.
    ///
    /// `blocks` must contain the summary's complete epoch branch in execution
    /// order, including its terminal and excluding its previous terminal.
    ///
    /// Runs before [`Self::store_summary`] so the summary is the last durable step.
    fn merge_epoch_data(
        &self,
        summary: &EpochSummary,
        blocks: &[OLBlockCommitment],
    ) -> WorkerResult<()>;

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

    /// Fetches the stored ASM manifest at L1 height `height`, or `None` if none
    /// is stored.
    fn fetch_l1_manifest(&self, height: u32) -> WorkerResult<Option<AsmManifest>>;

    /// Fetches the stored ASM manifest of the L1 block `block`, or `None` if
    /// none is stored.
    ///
    /// Unlike [`fetch_l1_manifest`](Self::fetch_l1_manifest), this finds the
    /// manifest of a block that a reorg removed from the canonical chain, and
    /// never returns another block's manifest at the same height.
    fn fetch_l1_block_manifest(
        &self,
        block: &L1BlockCommitment,
    ) -> WorkerResult<Option<AsmManifest>>;

    /// Applies epoch-granular state index writes for a reconstructed epoch,
    /// derived from the epoch's execution `output`.
    fn apply_epoch_indexing(
        &self,
        epoch: &EpochCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()>;
}
