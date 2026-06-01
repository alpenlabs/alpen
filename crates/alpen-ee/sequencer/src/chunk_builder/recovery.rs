//! Chunk builder startup recovery.
//!
//! On startup the chunk builder must:
//! 1. Verify existing chunks are consistent with batches (orphan cleanup).
//! 2. Backfill any blocks that were batched but not yet chunked.
//!
//! Orphan cleanup runs synchronously at startup. Backfill runs in
//! `on_launch` and pushes entries into the processing queue — the
//! normal tick loop handles sealing, data fetching, and witness dispatch.

use alpen_ee_common::{BatchStorage, BlockNumHash, ChunkStorage, ExecBlockStorage};
use eyre::{eyre, Result};
use strata_acct_types::Hash;
use tracing::{debug, info, warn};

use super::state::{ChunkBuilderState, PendingEntry};
use crate::policy::AccumulationPolicy;

/// Revert chunks to the last complete batch boundary.
///
/// Walks backward from the latest chunk to find one whose `last_block`
/// matches its batch's `last_block` (i.e., it sits at a batch boundary),
/// then reverts all chunks after it. A chunk is reverted if:
///
/// - Its batch no longer exists (reorg reverted the batch but a crash prevented the chunk builder
///   from reverting its chunks).
/// - It does not end at its batch's boundary (crash mid-batch). These chunks are discarded because
///   the sealing policy may produce a different batch after restart, making the old chunks invalid.
pub async fn cleanup_orphaned_chunks(
    chunk_storage: &impl ChunkStorage,
    batch_storage: &impl BatchStorage,
) -> Result<()> {
    let Some((latest_chunk, _)) = chunk_storage.get_latest_chunk().await? else {
        return Ok(()); // no chunks, nothing to clean
    };

    let (latest_batch, _) = batch_storage
        .get_latest_batch()
        .await?
        .ok_or_else(|| eyre!("no batches in storage; genesis batch expected"))?;

    // Walk backward to find the latest chunk that sits at a batch boundary.
    let mut revert_from = 0;
    for idx in (0..=latest_chunk.idx()).rev() {
        let (chunk, _) = chunk_storage
            .get_chunk_by_idx(idx)
            .await?
            .ok_or_else(|| eyre!("chunk at idx {idx} missing; storage may be corrupted"))?;

        if chunk.batch_idx() > latest_batch.idx() {
            continue;
        }

        let Some((batch, _)) = batch_storage.get_batch_by_idx(chunk.batch_idx()).await? else {
            continue;
        };

        // Chunk ends at this batch's boundary — keep it and everything before.
        if chunk.last_block() == batch.last_block() {
            revert_from = idx + 1;
            break;
        }
    }

    if revert_from > latest_chunk.idx() {
        return Ok(()); // nothing to revert
    }

    warn!(
        revert_from,
        latest_chunk_idx = latest_chunk.idx(),
        latest_batch_idx = latest_batch.idx(),
        "reverting chunks past last complete batch boundary"
    );
    chunk_storage.revert_chunks_from(revert_from).await?;
    Ok(())
}

/// Repair batch-chunk linkage for the latest chunk's batch if missing.
///
/// After [`cleanup_orphaned_chunks`], the latest chunk always sits at a
/// batch boundary. If the linkage (`set_batch_chunks`) was not persisted
/// before a crash, reconstruct it from the chunks' `batch_idx` fields.
pub async fn repair_batch_linkage(
    chunk_storage: &impl ChunkStorage,
    batch_storage: &impl BatchStorage,
) -> Result<()> {
    let Some((latest_chunk, _)) = chunk_storage.get_latest_chunk().await? else {
        return Ok(());
    };

    let (batch, _) = batch_storage
        .get_batch_by_idx(latest_chunk.batch_idx())
        .await?
        .ok_or_else(|| {
            eyre!(
                "batch {} for latest chunk {} not found after cleanup",
                latest_chunk.batch_idx(),
                latest_chunk.idx()
            )
        })?;

    let batch_id = batch.id();

    if chunk_storage.get_batch_chunks(batch_id).await?.is_some() {
        return Ok(()); // already linked
    }

    // Collect chunk IDs for this batch by walking backward from the latest chunk.
    let mut chunk_ids = Vec::new();
    for idx in (0..=latest_chunk.idx()).rev() {
        let (chunk, _) = chunk_storage
            .get_chunk_by_idx(idx)
            .await?
            .ok_or_else(|| eyre!("chunk at idx {idx} missing; storage may be corrupted"))?;
        if chunk.batch_idx() != latest_chunk.batch_idx() {
            break;
        }
        chunk_ids.push(chunk.id());
    }
    chunk_ids.reverse();

    info!(
        batch_idx = latest_chunk.batch_idx(),
        chunk_count = chunk_ids.len(),
        "repaired missing batch-chunk linkage"
    );
    chunk_storage
        .set_batch_chunks(batch_id, chunk_ids)
        .await
        .map_err(|e| eyre!("set_batch_chunks: {e}"))?;
    Ok(())
}

/// Enqueue pending entries for blocks that were batched but not yet chunked.
///
/// Walks sealed batches from the chunk builder's current position forward
/// and pushes `PendingEntry::Block` and `PendingEntry::BatchBoundary`
/// entries into the processing queue. The normal tick loop handles
/// data fetching, sealing, and witness dispatch.
pub(crate) async fn enqueue_backfill<P: AccumulationPolicy>(
    state: &mut ChunkBuilderState<P>,
    batch_storage: &impl BatchStorage,
    block_storage: &impl ExecBlockStorage,
) -> Result<()> {
    let (latest_batch, _) = batch_storage
        .get_latest_batch()
        .await?
        .ok_or_else(|| eyre!("no batches in storage; genesis batch expected"))?;

    let last_chunked = state.prev_chunk_end();
    let start_batch_idx = state.current_batch_idx();

    if last_chunked.hash() == latest_batch.last_block() {
        debug!("chunk builder is caught up with latest batch");
        return Ok(());
    }

    info!(
        start_batch_idx,
        latest_batch_idx = latest_batch.idx(),
        last_chunked = %last_chunked.hash(),
        "enqueuing backfill for unchunked batches"
    );

    let mut last_pos = last_chunked;
    let mut enqueued = 0usize;

    for batch_idx in start_batch_idx..=latest_batch.idx() {
        let (batch, _) = batch_storage
            .get_batch_by_idx(batch_idx)
            .await?
            .ok_or_else(|| eyre!("batch {batch_idx} not found during backfill"))?;

        // Skip if we're already past this batch.
        if batch.last_block() == last_pos.hash() {
            last_pos = batch.last_blocknumhash();
            continue;
        }

        // Walk the chain from our current position to this batch's end.
        let blocks = get_block_range(last_pos.hash(), batch.last_block(), block_storage).await?;

        for block in blocks {
            state.push_pending(PendingEntry::Block { block, batch_idx });
            enqueued += 1;
        }

        state.push_pending(PendingEntry::BatchBoundary(batch.id()));

        last_pos = batch.last_blocknumhash();
    }

    info!(enqueued, "backfill entries enqueued");
    Ok(())
}

/// Walk the chain backward from `to_hash` to `from_hash` (exclusive)
/// and return the blocks in forward order.
async fn get_block_range(
    from_hash: Hash,
    to_hash: Hash,
    block_storage: &impl ExecBlockStorage,
) -> Result<Vec<BlockNumHash>> {
    if from_hash == to_hash {
        return Ok(Vec::new());
    }

    let from_block = block_storage
        .get_exec_block(from_hash)
        .await
        .map_err(|e| eyre!("get_exec_block({from_hash}): {e}"))?
        .ok_or_else(|| eyre!("block not found: {from_hash}"))?;

    let mut blocks = Vec::new();
    let mut current_hash = to_hash;

    while current_hash != from_hash {
        let current_block = block_storage
            .get_exec_block(current_hash)
            .await
            .map_err(|e| eyre!("get_exec_block({current_hash}): {e}"))?
            .ok_or_else(|| eyre!("block not found: {current_hash}"))?;

        if current_block.blocknum() < from_block.blocknum() {
            return Err(eyre!(
                "to_hash ({to_hash}) does not extend from_hash ({from_hash})"
            ));
        }

        blocks.push(current_block.blocknumhash());
        current_hash = current_block.parent_blockhash();
    }

    blocks.reverse();
    Ok(blocks)
}
