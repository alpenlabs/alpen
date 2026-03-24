//! Context trait for checkpoint worker dependencies.

use std::sync::Arc;

use strata_checkpoint_types::EpochSummary;
use strata_db_types::types::OLCheckpointEntry;
use strata_identifiers::{Epoch, OLBlockCommitment};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, OLBlockId, OLLog};
use strata_ol_state_support_types::DaAccumulatingState;
use strata_ol_state_types::OLState;
use strata_ol_stf::execute_block_batch;
use strata_primitives::{epoch::EpochCommitment, nonempty_vec::NonEmptyVec};
use strata_storage::NodeStorage;

pub(crate) type StateDiffRaw = Vec<u8>;

/// Context providing dependencies for the checkpoint worker.
///
/// This trait abstracts storage and data providers, enabling testing
/// with mock implementations and future production providers.
pub(crate) trait CheckpointWorkerContext: Send + Sync + 'static {
    /// Get the last summarized epoch index, if any.
    fn get_last_summarized_epoch(&self) -> anyhow::Result<Option<u64>>;

    /// Get the canonical epoch commitment for a given epoch index.
    fn get_canonical_epoch_commitment_at(
        &self,
        index: u64,
    ) -> anyhow::Result<Option<EpochCommitment>>;

    /// Get the epoch summary for a commitment.
    fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<EpochSummary>>;

    /// Get a checkpoint entry for the given epoch.
    fn get_checkpoint(&self, epoch: Epoch) -> anyhow::Result<Option<OLCheckpointEntry>>;

    /// Get the last checkpointed epoch, if any.
    fn get_last_checkpoint_epoch(&self) -> anyhow::Result<Option<Epoch>>;

    /// Store a checkpoint entry for the given epoch.
    fn put_checkpoint(&self, epoch: Epoch, entry: OLCheckpointEntry) -> anyhow::Result<()>;

    /// Gets proof bytes for the checkpoint.
    fn get_proof(&self, epoch: &EpochCommitment) -> anyhow::Result<Vec<u8>>;

    /// Gets the OL block header for the given block id.
    fn get_block_header(&self, blkid: &OLBlockCommitment) -> anyhow::Result<Option<OLBlockHeader>>;

    /// Gets an OL block by its block ID.
    fn get_block(&self, id: &OLBlockId) -> anyhow::Result<Option<OLBlock>>;

    /// Gets the OL state snapshot at a given block commitment.
    fn get_ol_state(&self, commitment: &OLBlockCommitment) -> anyhow::Result<Option<OLState>>;

    /// Fetches da data for epoch. Returns state diff and OL logs.
    fn fetch_da_for_epoch(
        &self,
        summary: &EpochSummary,
    ) -> anyhow::Result<(StateDiffRaw, Vec<OLLog>)>;
}

/// Production context implementation with v1 defaults.
///
/// Uses empty DA, empty logs, and placeholder proof.
pub(crate) struct CheckpointWorkerContextImpl {
    storage: Arc<NodeStorage>,
}

impl CheckpointWorkerContextImpl {
    /// Create a new context with the given storage.
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }
}

impl CheckpointWorkerContext for CheckpointWorkerContextImpl {
    fn get_last_summarized_epoch(&self) -> anyhow::Result<Option<u64>> {
        self.storage
            .ol_checkpoint()
            .get_last_summarized_epoch_blocking()
            .map_err(Into::into)
    }

    fn get_canonical_epoch_commitment_at(
        &self,
        index: u64,
    ) -> anyhow::Result<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_blocking(index)
            .map_err(Into::into)
    }

    fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<EpochSummary>> {
        self.storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(commitment)
            .map_err(Into::into)
    }

    fn get_checkpoint(&self, epoch: Epoch) -> anyhow::Result<Option<OLCheckpointEntry>> {
        self.storage
            .ol_checkpoint()
            .get_checkpoint_blocking(epoch)
            .map_err(Into::into)
    }

    fn get_last_checkpoint_epoch(&self) -> anyhow::Result<Option<Epoch>> {
        self.storage
            .ol_checkpoint()
            .get_last_checkpoint_epoch_blocking()
            .map_err(Into::into)
    }

    fn put_checkpoint(&self, epoch: Epoch, entry: OLCheckpointEntry) -> anyhow::Result<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_blocking(epoch, entry)
            .map_err(Into::into)
    }

    fn get_proof(&self, _epoch: &EpochCommitment) -> anyhow::Result<Vec<u8>> {
        // V1: empty placeholder proof
        Ok(Vec::new())
    }

    fn get_block_header(
        &self,
        terminal: &OLBlockCommitment,
    ) -> anyhow::Result<Option<OLBlockHeader>> {
        let maybe_block = self
            .storage
            .ol_block()
            .get_block_data_blocking(*terminal.blkid())?;
        Ok(maybe_block.map(|block| block.header().clone()))
    }

    fn get_block(&self, id: &OLBlockId) -> anyhow::Result<Option<OLBlock>> {
        self.storage
            .ol_block()
            .get_block_data_blocking(*id)
            .map_err(Into::into)
    }

    fn get_ol_state(&self, commitment: &OLBlockCommitment) -> anyhow::Result<Option<OLState>> {
        let state = self
            .storage
            .ol_state()
            .get_toplevel_ol_state_blocking(*commitment)?;
        Ok(state.map(|arc| (*arc).clone()))
    }

    fn fetch_da_for_epoch(
        &self,
        summary: &EpochSummary,
    ) -> anyhow::Result<(StateDiffRaw, Vec<OLLog>)> {
        let (statediff, logs, terminal_header) = replay_epoch_and_compute_da(self, summary)?;
        assert_terminal_commitment_matches(&terminal_header, summary.terminal())?;
        Ok((statediff, logs))
    }
}

fn assert_terminal_commitment_matches(
    terminal_header: &OLBlockHeader,
    expected_terminal: &OLBlockCommitment,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        terminal_header.slot() == expected_terminal.slot(),
        "terminal header slot mismatch: expected {}, got {}",
        expected_terminal.slot(),
        terminal_header.slot()
    );
    anyhow::ensure!(
        terminal_header.compute_blkid() == *expected_terminal.blkid(),
        "terminal header block id mismatch: expected {:?}, got {:?}",
        expected_terminal.blkid(),
        terminal_header.compute_blkid()
    );
    Ok(())
}

/// Replays epoch blocks to produce DA state diff bytes, accumulated logs, and
/// the terminal header.
///
/// Loads the OL state at the previous terminal block, wraps it in
/// `DaAccumulatingState` to intercept mutations, then re-executes every block
/// in the epoch. The DA blob is extracted from the accumulating layer and the
/// logs are collected from each block's execution output.
fn replay_epoch_and_compute_da<C: CheckpointWorkerContext>(
    ctx: &C,
    summary: &EpochSummary,
) -> anyhow::Result<(Vec<u8>, Vec<OLLog>, OLBlockHeader)> {
    let epoch_blocks = collect_epoch_blocks(summary, ctx)?;

    let prev_terminal = summary.prev_terminal();
    let prev_terminal_header = ctx.get_block_header(prev_terminal)?.ok_or_else(|| {
        anyhow::anyhow!("missing prev terminal block header for {:?}", prev_terminal)
    })?;

    let ol_state = ctx
        .get_ol_state(prev_terminal)?
        .ok_or_else(|| anyhow::anyhow!("missing OL state at prev terminal {:?}", prev_terminal))?;

    let mut da_state = DaAccumulatingState::new(ol_state);

    let logs = execute_block_batch(&mut da_state, &epoch_blocks, &prev_terminal_header)
        .map_err(|e| anyhow::anyhow!("epoch block replay failed: {e}"))?;

    let terminal_header = epoch_blocks.ensured_last().header().clone();

    // Extract the DA blob from the accumulating layer.
    let da_bytes = da_state
        .take_completed_epoch_da_blob()
        .map_err(|e| anyhow::anyhow!("DA accumulation failed: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("no DA blob produced after epoch replay"))?;

    Ok((da_bytes, logs, terminal_header))
}

/// Collects all blocks in an epoch by walking backwards from the terminal block.
///
/// Returns blocks in forward order (first block of epoch first, terminal last).
fn collect_epoch_blocks<C: CheckpointWorkerContext>(
    summary: &EpochSummary,
    ctx: &C,
) -> anyhow::Result<NonEmptyVec<OLBlock>> {
    let terminal_blkid = summary.terminal().blkid();
    let prev_terminal_blkid = summary.prev_terminal().blkid();
    let prev_terminal_slot = summary.prev_terminal().slot();

    let mut blocks = Vec::new();
    let mut cur_id = *terminal_blkid;

    loop {
        let block = ctx
            .get_block(&cur_id)?
            .ok_or_else(|| anyhow::anyhow!("missing block {cur_id:?} while collecting epoch"))?;

        anyhow::ensure!(
            block.header().slot() > prev_terminal_slot,
            "block at slot {} is at or below prev terminal slot {}; \
             epoch chain is broken",
            block.header().slot(),
            prev_terminal_slot,
        );

        // Check if the same epoch is being traversed.
        anyhow::ensure!(
            block.header().epoch() == summary.epoch(),
            "Obtained a block with different epoch, expected {}, obtained {}",
            summary.epoch(),
            block.header().epoch(),
        );

        let parent_id = *block.header().parent_blkid();
        blocks.push(block);

        if parent_id == *prev_terminal_blkid {
            break;
        }

        cur_id = parent_id;
    }

    blocks.reverse();
    let blocks =
        NonEmptyVec::try_from_vec(blocks).map_err(|_| anyhow::anyhow!("Non-empty epoch blocks"))?;
    Ok(blocks)
}
