//! Batch builder task implementation.

use std::time::Duration;

use alpen_ee_common::{Batch, BatchId, BatchStorage, ExecBlockStorage};
use alpen_ee_exec_chain::ExecChainHandle;
use eyre::Result;
use strata_acct_types::Hash;
use tokio::time;
use tracing::{debug, error, warn};

use super::{
    ctx::BatchBuilderCtx, BatchBuilderState, BatchPolicy, BatchSealingPolicy, BlockDataProvider,
};

/// Check if a block is on the canonical chain (finalized or unfinalized canonical).
async fn is_block_canonical(
    hash: Hash,
    exec_chain: &ExecChainHandle,
    block_storage: &impl ExecBlockStorage,
) -> Result<bool> {
    // Check finalized first (avoids async query to exec_chain)
    if block_storage.get_finalized_height(hash).await?.is_some() {
        return Ok(true);
    }
    // Check unfinalized canonical chain
    exec_chain.is_canonical(hash).await
}

/// Find the last batch whose end block is still canonical.
///
/// Returns `(batch_idx, last_block_hash, batch_id)` or `None` if no batches exist or none are canonical.
async fn find_last_canonical_batch(
    exec_chain: &ExecChainHandle,
    batch_storage: &impl BatchStorage,
    block_storage: &impl ExecBlockStorage,
) -> Result<Option<(u64, Hash, BatchId)>> {
    let Some((batch, _status)) = batch_storage.get_latest_batch().await? else {
        return Ok(None);
    };

    let mut idx = batch.idx();
    loop {
        let Some((batch, _)) = batch_storage.get_batch_by_idx(idx).await? else {
            return Ok(None);
        };

        if is_block_canonical(batch.last_block(), exec_chain, block_storage).await? {
            return Ok(Some((idx, batch.last_block(), batch.id())));
        }

        if idx == 0 {
            return Ok(None);
        }
        idx -= 1;
    }
}

/// Check for reorgs and handle them.
///
/// Returns `Some(new_latest_batch_id)` if a deep reorg was detected and batches were reverted.
/// The inner `Option<BatchId>` is `None` if all batches were reverted, `Some(id)` otherwise.
/// Returns `None` if no deep reorg occurred (including shallow reorgs).
async fn check_and_handle_reorg<P: BatchPolicy>(
    state: &mut BatchBuilderState<P>,
    exec_chain: &ExecChainHandle,
    block_storage: &impl ExecBlockStorage,
    batch_storage: &impl BatchStorage,
    genesis_hash: Hash,
) -> Result<Option<Option<BatchId>>> {
    // Check if prev_batch_end is still canonical
    if !is_block_canonical(state.prev_batch_end(), exec_chain, block_storage).await? {
        // Deep reorg - find last canonical batch
        if let Some((idx, last_block, batch_id)) =
            find_last_canonical_batch(exec_chain, batch_storage, block_storage).await?
        {
            // Revert batches after the canonical one
            batch_storage.revert_batch(idx).await?;
            *state = BatchBuilderState::from_last_batch(idx, last_block);
            warn!(
                reverted_to_idx = idx,
                "Deep reorg detected, reverted batches"
            );
            return Ok(Some(Some(batch_id)));
        } else {
            // No canonical batches - revert all and reset to genesis
            batch_storage.revert_batch(0).await?;
            *state = BatchBuilderState::from_genesis(genesis_hash);
            warn!("Deep reorg detected, reverted all batches to genesis");
            return Ok(Some(None));
        }
    }

    // Check shallow reorg in accumulator (doesn't affect sealed batches)
    for hash in state.accumulator().blocks() {
        if !is_block_canonical(*hash, exec_chain, block_storage).await? {
            state.accumulator_mut().reset();
            state.clear_pending_blocks();
            debug!("Shallow reorg detected, reset accumulator and pending blocks");
            // No notification needed - latest batch unchanged
            return Ok(None);
        }
    }

    Ok(None)
}

/// Get block hashes from `from_hash` (exclusive) to `to_hash` (inclusive).
///
/// Walks backwards from `to_hash` until reaching `from_hash`.
async fn get_block_range(
    from_hash: Hash,
    to_hash: Hash,
    block_storage: &impl ExecBlockStorage,
) -> Result<Vec<Hash>> {
    let mut blocks = Vec::new();
    let mut current = to_hash;

    while current != from_hash {
        blocks.push(current);
        let block = block_storage
            .get_exec_block(current)
            .await?
            .ok_or_else(|| eyre::eyre!("Block not found: {}", current))?;
        current = block.parent_blockhash();
    }

    blocks.reverse();
    Ok(blocks)
}

/// Seal the current batch.
///
/// Returns the sealed batch ID, or `None` if accumulator was empty.
async fn seal_batch<P: BatchPolicy>(
    state: &mut BatchBuilderState<P>,
    batch_storage: &impl BatchStorage,
) -> Result<Option<BatchId>> {
    if state.accumulator().is_empty() {
        return Ok(None);
    }

    let prev_block = state.prev_batch_end();
    let (inner_blocks, last_block) = state.accumulator_mut().drain_for_batch();

    let batch_idx = state.next_batch_idx();
    let batch = Batch::new(batch_idx, prev_block, last_block, inner_blocks);
    let batch_id = batch.id();

    debug!(
        batch_idx = batch.idx(),
        prev_block = %prev_block,
        last_block = %last_block,
        "Sealing batch"
    );

    batch_storage.save_next_batch(batch).await?;
    state.advance_batch(last_block);

    Ok(Some(batch_id))
}

/// Check if the first pending block has data available.
///
/// Returns `Some(block_data)` if the first pending block exists and has data ready,
/// otherwise returns `None`. This is used to gate the processing branch in the select.
async fn check_first_pending_block_data<P: BatchPolicy, D: BlockDataProvider<P>>(
    state: &BatchBuilderState<P>,
    block_data_provider: &D,
) -> Option<P::BlockData> {
    let hash = state.first_pending_block()?;
    // Assumes data lookup is cached or cheap to recompute
    block_data_provider.get_block_data(hash).await.ok()?
}

/// Main batch builder task.
///
/// This task monitors the canonical chain and builds batches according to the
/// sealing policy. It handles reorgs and persists sealed batches to storage.
///
/// The task uses two concurrent branches:
/// 1. React to new canonical tips, check for reorgs, and queue unprocessed blocks
/// 2. Process blocks from the queue when their data becomes available
pub(crate) async fn batch_builder_task<P, D, S, BS, ES>(
    mut state: BatchBuilderState<P>,
    mut ctx: BatchBuilderCtx<P, D, S, BS, ES>,
) where
    P: BatchPolicy,
    D: BlockDataProvider<P>,
    S: BatchSealingPolicy<P>,
    BS: BatchStorage,
    ES: ExecBlockStorage,
{
    // TODO: backoff logic
    let error_backoff = Duration::from_millis(ctx.config.error_backoff_ms);

    loop {
        let result = tokio::select! {
            // Branch 1: New canonical tip received
            changed = ctx.preconf_rx.changed() => {
                if changed.is_err() {
                    warn!("preconf_rx channel closed; exiting");
                    return;
                }
                let new_tip = *ctx.preconf_rx.borrow_and_update();
                handle_new_tip(&mut state, &ctx, new_tip).await
            }

            // Branch 2: Process pending blocks when first block's data is ready
            Some(_) = check_first_pending_block_data(&state, ctx.block_data_provider.as_ref()) => {
                process_pending_blocks(&mut state, &ctx).await
            }
        };

        if let Err(e) = result {
            error!(error = %e, "Batch builder error, backing off");
            time::sleep(error_backoff).await;
        }
    }
}

/// Handle a new canonical tip update.
///
/// Checks for reorgs and queues any new blocks for processing.
async fn handle_new_tip<P, D, S, BS, ES>(
    state: &mut BatchBuilderState<P>,
    ctx: &BatchBuilderCtx<P, D, S, BS, ES>,
    new_tip: Hash,
) -> Result<()>
where
    P: BatchPolicy,
    D: BlockDataProvider<P>,
    S: BatchSealingPolicy<P>,
    BS: BatchStorage,
    ES: ExecBlockStorage,
{
    // Check and handle reorgs first
    if let Some(new_latest) = check_and_handle_reorg(
        state,
        &ctx.exec_chain,
        ctx.block_storage.as_ref(),
        ctx.batch_storage.as_ref(),
        ctx.genesis_hash,
    )
    .await?
    {
        // Deep reorg occurred, notify watchers
        let _ = ctx.latest_batch_tx.send(new_latest);
        // State was reset, pending blocks were cleared
        // Fall through to queue new blocks from the reset point
    }

    // Determine starting point for fetching new blocks
    let start_hash = state.last_known_block();

    // Get blocks from start to new tip and add to pending queue
    let blocks = get_block_range(start_hash, new_tip, ctx.block_storage.as_ref()).await?;

    if !blocks.is_empty() {
        debug!(
            count = blocks.len(),
            start = %start_hash,
            tip = %new_tip,
            "Queuing new blocks"
        );
        state.push_pending_blocks(blocks);
    }

    Ok(())
}

/// Process pending blocks whose data is ready.
///
/// Processes blocks sequentially from the front of the queue. Stops when
/// a block's data is not yet available or the queue is empty.
async fn process_pending_blocks<P, D, S, BS, ES>(
    state: &mut BatchBuilderState<P>,
    ctx: &BatchBuilderCtx<P, D, S, BS, ES>,
) -> Result<()>
where
    P: BatchPolicy,
    D: BlockDataProvider<P>,
    S: BatchSealingPolicy<P>,
    BS: BatchStorage,
    ES: ExecBlockStorage,
{
    // Process blocks while data is available
    while let Some(hash) = state.first_pending_block() {
        // Try to get block data (non-blocking check)
        let Some(block_data) = ctx.block_data_provider.get_block_data(hash).await? else {
            // Data not ready yet, stop processing
            break;
        };

        // Data is ready, remove from pending queue
        state.pop_pending_block();

        // Check if adding this block would exceed threshold
        if !state.accumulator().is_empty()
            && ctx
                .sealing_policy
                .would_exceed(state.accumulator(), &block_data)
        {
            if let Some(batch_id) = seal_batch(state, ctx.batch_storage.as_ref()).await? {
                // Notify watchers of new batch
                let _ = ctx.latest_batch_tx.send(Some(batch_id));
            }
        }

        // Add block to accumulator
        state.accumulator_mut().add_block(hash, &block_data);

        debug!(hash = %hash, "Processed block");
    }

    Ok(())
}
