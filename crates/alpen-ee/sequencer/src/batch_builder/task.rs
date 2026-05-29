//! Batch builder task implementation.

use std::time::Duration;

use alpen_ee_common::{
    Batch, BatchId, BatchStorage, BlockNumHash, Chunk, ChunkWitnessStore, ExecBlockStorage,
};
use eyre::{eyre, Result};
use strata_acct_types::Hash;
use tokio::{sync::mpsc, time};
use tracing::{debug, error, warn};

use super::{
    ctx::BatchBuilderCtx, BatchBuilderState, BatchPolicy, BatchSealingPolicy, BlockDataProvider,
};
use crate::{
    batch_builder::reorg::{check_and_handle_reorg, ReorgReport},
    chunk_witness_task::ChunkExtractRequest,
};

/// Polling interval for checking pending block data availability.
const PENDING_BLOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Maximum number of blocks to process in a single polling cycle.
/// This prevents blocking the select loop for too long when many blocks have data ready.
const MAX_BLOCKS_PER_CYCLE: usize = 10;

/// Get block hashes and heights from `from_hash` (exclusive) to `to_hash` (inclusive).
///
/// Walks backwards from `to_hash` until reaching `from_hash`.
/// Returns an empty vec if `from_hash == to_hash`.
async fn get_block_range(
    from_hash: Hash,
    to_hash: Hash,
    block_storage: &impl ExecBlockStorage,
) -> Result<Vec<BlockNumHash>> {
    // Ensure endpoint exists
    let from_block = block_storage
        .get_exec_block(from_hash)
        .await?
        .ok_or_else(|| eyre::eyre!("Block not found: from_hash = {}", from_hash))?;

    if from_hash == to_hash {
        return Ok(Vec::new());
    }

    let mut blocks = Vec::new();
    let mut current_hash = to_hash;

    while current_hash != from_hash {
        let current_block = block_storage
            .get_exec_block(current_hash)
            .await?
            .ok_or_else(|| eyre::eyre!("Block not found: {}", current_hash))?;

        if current_block.blocknum() < from_block.blocknum() {
            return Err(eyre!(
                "to_hash ({}) does not extend from_hash ({})",
                to_hash,
                from_hash
            ));
        }

        blocks.push(current_block.blocknumhash());
        current_hash = current_block.parent_blockhash();
    }

    blocks.reverse();
    Ok(blocks)
}

/// Seal the current batch.
///
/// Returns the sealed batch ID, or `None` if accumulator was empty.
///
/// When `chunk_witness_tx` is provided, publishes a
/// [`ChunkExtractRequest`] on the channel for the background
/// `chunk_witness_task` to process. Extraction itself runs off this
/// task's hot path; sealing does not wait for it.
async fn seal_batch<P: BatchPolicy>(
    state: &mut BatchBuilderState<P>,
    storage: &(impl BatchStorage + ChunkWitnessStore),
    chunk_witness_tx: Option<&mpsc::Sender<ChunkExtractRequest>>,
) -> Result<Option<BatchId>> {
    if state.accumulator().is_empty() {
        return Ok(None);
    }

    let prev_block = state.prev_batch_end();
    let (inner_blocks, last_block) = state.accumulator_mut().drain_for_batch();
    let inner_blocks: Vec<Hash> = inner_blocks.into_iter().map(|b| b.hash()).collect();

    let batch_idx = state.next_batch_idx();
    let batch = Batch::new(
        batch_idx,
        prev_block.hash(),
        last_block.hash(),
        last_block.blocknum(),
        inner_blocks.clone(),
    )
    .map_err(|err| eyre!(err))?;
    let batch_id = batch.id();

    debug!(
        batch_idx = batch.idx(),
        prev_block = %prev_block.hash(),
        last_block = %last_block.hash(),
        "Sealing batch"
    );

    storage.save_next_batch(batch).await?;

    // One chunk per batch, spanning the whole batch.
    //
    // TODO(STR-1369): replace with a real chunking/batching policy
    // (e.g. sub-batch chunker driven by prover cost). Today the PAAS
    // chunk + acct provers only need the chunk records to exist and
    // be linked to the batch — cardinality is a policy knob.
    let next_chunk_idx = storage
        .get_latest_chunk()
        .await?
        .map(|(c, _)| c.idx() + 1)
        .unwrap_or(0);
    let chunk = Chunk::new(
        next_chunk_idx,
        prev_block.hash(),
        last_block.hash(),
        inner_blocks,
    );
    let chunk_id = chunk.id();
    // The chunk's first block (inner_blocks[0] if non-empty, else
    // last_block). The extractor needs this — NOT `chunk_id.prev_block`,
    // which is the last block of the *previous* chunk and lives in reth
    // as a block, not as the chunk's range start.
    let first_block_hash = chunk
        .blocks_iter()
        .next()
        .expect("chunk has at least last_block");
    let last_block_hash = chunk.last_block();
    storage.save_next_chunk(chunk).await?;
    storage.set_batch_chunks(batch_id, vec![chunk_id]).await?;

    if let Some(tx) = chunk_witness_tx {
        // Hand the chunk off to the background witness task. `send`
        // backpressures the builder if the extractor is sustainedly
        // behind (channel full), but does not block on the per-chunk
        // extraction itself. Channel closure means the task has died —
        // log so it's visible, but keep sealing; chunks without a
        // witness produce `TransientFailure` at proof time and can be
        // backfilled later.
        let req = ChunkExtractRequest {
            chunk_id,
            first_block: first_block_hash,
            last_block: last_block_hash,
        };
        if let Err(e) = tx.send(req).await {
            warn!(
                ?chunk_id,
                error = %e,
                "chunk witness channel closed; chunk sealed without witness — \
                 chunk will remain proof-blocked until manually backfilled"
            );
        }
    }

    state.advance_batch(last_block);

    Ok(Some(batch_id))
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
    BS: BatchStorage + ChunkWitnessStore,
    ES: ExecBlockStorage,
{
    let mut pending_poll_interval = time::interval(PENDING_BLOCK_POLL_INTERVAL);

    loop {
        let result = tokio::select! {
            // Branch 1: New canonical tip received
            changed = ctx.preconf_rx.changed() => {
                if changed.is_err() {
                    warn!("preconf_rx channel closed; exiting");
                    return;
                }
                let new_tip = *ctx.preconf_rx.borrow_and_update();
                debug!("canonical tip received: {:?}", new_tip );
                handle_new_tip(&mut state, &ctx, new_tip).await
            }

            // Branch 2: Periodically poll pending blocks when queue is non-empty
            _ = pending_poll_interval.tick(), if state.has_pending_blocks() => {
                debug!("processing pending blocks");
                process_pending_blocks(&mut state, &ctx).await
            }
        };

        if let Err(e) = result {
            error!(error = %e, "Batch builder error");
        }
    }
}

/// Handle a new canonical tip update.
///
/// Checks for reorgs and queues any new blocks for processing.
async fn handle_new_tip<P, D, S, BS, ES>(
    state: &mut BatchBuilderState<P>,
    ctx: &BatchBuilderCtx<P, D, S, BS, ES>,
    new_tip: BlockNumHash,
) -> Result<()>
where
    P: BatchPolicy,
    D: BlockDataProvider<P>,
    S: BatchSealingPolicy<P>,
    BS: BatchStorage + ChunkWitnessStore,
    ES: ExecBlockStorage,
{
    // Check and handle reorgs first
    match check_and_handle_reorg(
        state,
        &ctx.canonical_reader(),
        ctx.batch_storage.as_ref(),
        ctx.genesis,
    )
    .await?
    {
        ReorgReport::NoReorg => {
            // No reorg detected.
            // Continue normal execution.
        }
        ReorgReport::ShallowReorg => {
            // Shallow reorg. Pending blocks and accumulator reset.
            // Latest batch has not changed.
            // Continue execution with new state.
        }
        ReorgReport::Reorg(batch_id) => {
            // Unfinalized batch is has been reorg'd.
            // Latest batch reverted. Pending blocks and accumulator reset.
            // notify new latest batch
            let _ = ctx.latest_batch_tx.send(batch_id);
            // Continue execution with new state.
        }
        ReorgReport::DeepReorg => {
            // TODO: unrecoverable error
            return Err(eyre!("deep reorg detected"));
        }
    }

    // Determine starting point for fetching new blocks
    let last_known = state.last_known_block();

    // Get blocks from start to new tip and add to pending queue
    let blocks = get_block_range(
        last_known.hash(),
        new_tip.hash(),
        ctx.block_storage.as_ref(),
    )
    .await?;

    if !blocks.is_empty() {
        debug!(
            count = blocks.len(),
            start = %last_known.hash(),
            tip = %new_tip.hash(),
            "Queuing new blocks"
        );
        state.push_pending_blocks(blocks);
    }

    Ok(())
}

/// Process pending blocks whose data is ready.
///
/// Processes blocks sequentially from the front of the queue. Stops when
/// a block's data is not yet available, the queue is empty, or the maximum
/// number of blocks per cycle is reached.
async fn process_pending_blocks<P, D, S, BS, ES>(
    state: &mut BatchBuilderState<P>,
    ctx: &BatchBuilderCtx<P, D, S, BS, ES>,
) -> Result<()>
where
    P: BatchPolicy,
    D: BlockDataProvider<P>,
    S: BatchSealingPolicy<P>,
    BS: BatchStorage + ChunkWitnessStore,
    ES: ExecBlockStorage,
{
    let mut processed = 0;

    // Process blocks while data is available, up to the max per cycle
    while processed < MAX_BLOCKS_PER_CYCLE {
        let Some(block) = state.first_pending_block() else {
            break;
        };

        // Try to get block data (non-blocking check)
        let Some(block_data) = ctx.block_data_provider.get_block_data(block.hash()).await? else {
            // Data not ready yet, stop processing
            println!(
                "processing pending blocks, block data not yet ready {:?}",
                block.hash()
            );
            break;
        };

        // Data is ready, remove from pending queue
        state.pop_pending_block();

        // Check if adding this block would exceed threshold
        if !state.accumulator().is_empty()
            && state
                .accumulator()
                .would_exceed(&ctx.sealing_policy, &block_data)
        {
            if let Some(batch_id) = seal_batch(
                state,
                ctx.batch_storage.as_ref(),
                ctx.chunk_witness_tx.as_ref(),
            )
            .await?
            {
                // Notify watchers of new batch
                let _ = ctx.latest_batch_tx.send(batch_id);
            }
        }

        // Add block to accumulator
        state.accumulator_mut().add_block(block, &block_data);

        debug!(hash = %block.hash(), "Processed block");
        processed += 1;
    }

    Ok(())
}
