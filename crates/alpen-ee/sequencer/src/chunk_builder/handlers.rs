//! Chunk builder processing logic.
//!
//! Called from the service framework's [`AsyncService::process_input`].
//! - `process_pending`: called on `TickMsg::Tick` — drains the pending queue, fetching block data
//!   and sealing chunks.
//! - `handle_reorg`: called on `TickMsg::Msg(Reorg { .. })`.

use alpen_ee_common::{BatchId, BatchStorage, BlockNumHash, Chunk, ChunkStorage, ExecBlockStorage};
use eyre::{eyre, Result};
use strata_acct_types::Hash;
use tracing::{debug, warn};

use super::{
    recovery,
    state::{ChunkBuilderState, PendingEntry},
};
use crate::sealing_policy::{AccumulationPolicy, BlockDataProvider, SealingPolicy};

/// Maximum number of entries to process in a single tick cycle.
const MAX_ENTRIES_PER_TICK: usize = 10;

/// Process pending entries from the queue.
///
/// Entries are either batch boundaries (processed immediately) or
/// blocks (require data from the provider; deferred if not ready).
///
/// Stops at the first block whose data is not ready, or after processing
/// [`MAX_ENTRIES_PER_TICK`] entries.
pub(crate) async fn process_pending<P, S, D>(
    state: &mut ChunkBuilderState<P>,
    chunk_storage: &impl ChunkStorage,
    sealing_policy: &S,
    block_data_provider: &D,
) -> Result<()>
where
    P: AccumulationPolicy,
    S: SealingPolicy<P>,
    D: BlockDataProvider<P>,
{
    let mut processed = 0;

    while processed < MAX_ENTRIES_PER_TICK {
        let Some(entry) = state.peek_pending().cloned() else {
            break;
        };

        match entry {
            PendingEntry::BatchBoundary(batch_id) => {
                state.pop_pending();

                // Skip if this batch is already processed (recovery or duplicate).
                if chunk_storage.get_batch_chunks(batch_id).await?.is_some() {
                    debug!(%batch_id, "skipping already-linked batch boundary");
                    processed += 1;
                    continue;
                }

                // Verify the sealed batch's last_block matches our position.
                // After force-sealing the accumulator, prev_chunk_end should
                // equal the batch's last_block. Check before sealing: the
                // accumulator's last block (or prev_chunk_end if empty) should
                // match the batch boundary.
                let expected_last = state
                    .accumulator()
                    .last_block()
                    .map(|b| b.hash())
                    .unwrap_or(state.prev_chunk_end().hash());
                if expected_last != batch_id.last_block() {
                    return Err(eyre!(
                        "batch boundary mismatch: chunk builder position={expected_last}, \
                         batch last_block={}",
                        batch_id.last_block()
                    ));
                }

                handle_batch_boundary(state, chunk_storage, batch_id).await?;
            }
            PendingEntry::Block { block, batch_idx } => {
                let next_expected = state.last_known_blocknum() + 1;

                if block.blocknum() < next_expected {
                    // Overlap: block already processed (recovery or duplicate).
                    state.pop_pending();
                    debug!(
                        blocknum = block.blocknum(),
                        next_expected, "skipping already-processed block"
                    );
                    processed += 1;
                    continue;
                }

                if block.blocknum() > next_expected {
                    // Gap: blocks are missing between our position and this event.
                    return Err(eyre!(
                        "gap detected: expected blocknum={next_expected} received blocknum={}",
                        block.blocknum()
                    ));
                }

                // Validate batch index is consistent with the batch builder's view.
                if batch_idx != state.current_batch_idx() {
                    return Err(eyre!(
                        "batch_idx mismatch: chunk builder has {}, event has {batch_idx}",
                        state.current_batch_idx()
                    ));
                }

                // Try to get block data; stop if not ready.
                let Some(block_data) = block_data_provider.get_block_data(block.hash()).await?
                else {
                    debug!(hash = %block.hash(), "block data not yet ready");
                    break;
                };

                state.pop_pending();

                if !state.accumulator().is_empty()
                    && state
                        .accumulator()
                        .would_exceed(sealing_policy, &block_data)
                {
                    seal_chunk(state, chunk_storage).await?;
                }

                state.accumulator_mut().add_block(block, &block_data);
                debug!(hash = %block.hash(), batch_idx, "chunk builder processed block");
            }
        }

        processed += 1;
    }

    Ok(())
}

/// Handle processing at a batch boundary.
///
/// Force-seals the current chunk (if non-empty), then persists the
/// batch-to-chunk association.
async fn handle_batch_boundary<P: AccumulationPolicy>(
    state: &mut ChunkBuilderState<P>,
    chunk_storage: &impl ChunkStorage,
    batch_id: BatchId,
) -> Result<()> {
    if !state.accumulator().is_empty() {
        seal_chunk(state, chunk_storage).await?;
    }

    let chunk_ids = state.take_batch_chunks();
    chunk_storage
        .set_batch_chunks(batch_id, chunk_ids)
        .await
        .map_err(|e| eyre!("set_batch_chunks: {e}"))?;
    debug!(%batch_id, "linked chunks to batch");

    // Advance to the next batch.
    state.set_current_batch_idx(state.current_batch_idx() + 1);

    Ok(())
}

/// Handle a reorg event from the batch builder.
///
/// Reorg events invalidate any queued block/boundary events because they were
/// emitted for the old canonical view. The handler therefore rebuilds the
/// in-memory frontier from storage, then enqueues backfill for any surviving
/// sealed batches that have not been chunked yet.
pub(crate) async fn handle_reorg<P: AccumulationPolicy>(
    state: &mut ChunkBuilderState<P>,
    chunk_storage: &impl ChunkStorage,
    batch_storage: &impl BatchStorage,
    block_storage: &impl ExecBlockStorage,
    revert_to: BlockNumHash,
    last_valid_batch_idx: u64,
) -> Result<()> {
    let (latest_batch, _) = batch_storage
        .get_latest_batch()
        .await?
        .ok_or_else(|| eyre!("no batches in storage; genesis batch expected"))?;

    if latest_batch.idx() != last_valid_batch_idx || latest_batch.last_block() != revert_to.hash() {
        warn!(
            latest_batch_idx = latest_batch.idx(),
            last_valid_batch_idx,
            latest_batch_last = %latest_batch.last_block(),
            revert_to = %revert_to.hash(),
            "chunk builder reorg event does not match latest batch storage"
        );
    }

    // Drop chunks from reverted batches or incomplete batch work, then repair
    // the boundary link if the process had sealed chunks but not linked them
    // before this reorg event was processed.
    recovery::cleanup_orphaned_chunks(chunk_storage, batch_storage).await?;
    recovery::repair_batch_linkage(chunk_storage, batch_storage).await?;

    reset_state_to_storage_frontier(state, chunk_storage, batch_storage).await?;
    recovery::enqueue_backfill(state, batch_storage, block_storage).await?;

    debug!(
        revert_to = %revert_to.hash(),
        prev_chunk_end = %state.prev_chunk_end().hash(),
        next_chunk_idx = state.next_chunk_idx(),
        current_batch_idx = state.current_batch_idx(),
        "chunk builder reorg handled"
    );

    Ok(())
}

async fn reset_state_to_storage_frontier<P: AccumulationPolicy>(
    state: &mut ChunkBuilderState<P>,
    chunk_storage: &impl ChunkStorage,
    batch_storage: &impl BatchStorage,
) -> Result<()> {
    *state = if let Some((chunk, _)) = chunk_storage.get_latest_chunk().await? {
        ChunkBuilderState::from_last_chunk(
            chunk.idx(),
            BlockNumHash::new(chunk.last_block(), chunk.last_blocknum()),
            chunk.batch_idx() + 1,
        )
    } else {
        let (genesis, _) = batch_storage
            .get_batch_by_idx(0)
            .await?
            .ok_or_else(|| eyre!("genesis batch missing from storage"))?;
        let mut state = ChunkBuilderState::new(genesis.last_blocknumhash());
        state.set_current_batch_idx(1);
        state
    };

    Ok(())
}

/// Seal the current chunk from the accumulator.
async fn seal_chunk<P: AccumulationPolicy>(
    state: &mut ChunkBuilderState<P>,
    chunk_storage: &impl ChunkStorage,
) -> Result<()> {
    let prev_block = state.prev_chunk_end();
    let (inner_blocks, last_block) = state.accumulator_mut().drain();
    let inner_block_hashes: Vec<Hash> = inner_blocks.into_iter().map(|b| b.hash()).collect();

    let chunk_idx = state.next_chunk_idx();
    let chunk = Chunk::new(
        chunk_idx,
        prev_block.hash(),
        last_block.hash(),
        last_block.blocknum(),
        state.current_batch_idx(),
        inner_block_hashes,
    );
    let chunk_id = chunk.id();

    debug!(
        chunk_idx,
        prev_block = %prev_block.hash(),
        last_block = %last_block.hash(),
        "sealing chunk"
    );

    chunk_storage
        .save_next_chunk(chunk)
        .await
        .map_err(|e| eyre!("save_next_chunk: {e}"))?;

    state.push_batch_chunk(chunk_id);
    state.advance_chunk(last_block);

    Ok(())
}
