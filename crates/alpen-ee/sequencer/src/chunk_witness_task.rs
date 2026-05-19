//! Background chunk-witness extraction task.
//!
//! Owns the chunk-witness pipeline that previously ran inline in the
//! batch builder's `seal_batch`. The batch builder now publishes a
//! [`ChunkExtractRequest`] on an mpsc channel and continues; this task
//! drains the channel, runs the (CPU-heavy) extractor in
//! `spawn_blocking`, retries on transient failures, and persists the
//! result via [`ChunkWitnessStore`].
//!
//! Why a separate task:
//! - **No race against `AccessedStateGenerator`.** The exex and the batch builder both react to
//!   canonical-chain events; the builder can seal a chunk for blocks the exex hasn't persisted yet.
//!   Retrying inline would block the builder loop. Here, the retry happens off the hot path —
//!   nobody upstream cares how long the exex takes to catch up.
//! - **No blocking the builder pipeline.** `seal_batch` is a channel `send`. Extraction work, RLP
//!   encoding, and sled writes all happen on this task's executor.
//!
//! Failure model: the only realistic transient cause of extractor
//! failure is the upstream exex lag (see analysis on PR #1822). Other
//! errors (reth state pruned, multiproof inconsistency, sled IO) are
//! structural and won't recover on retry. The task retries
//! unconditionally with bounded exponential backoff — non-transient
//! errors burn the retry budget and then surface as a permanent
//! `error!` log, identical to the prior inline-warn behavior but
//! without losing chunks to the millisecond race window.

use std::{sync::Arc, time::Duration};

use alpen_ee_common::{BatchStorage, ChunkId, ChunkWitnessExtractFn, ChunkWitnessStore};
use eyre::{eyre, Result};
use strata_acct_types::Hash;
use tokio::{sync::mpsc, task, time};
use tracing::{debug, error, info, instrument, warn, Instrument};

/// Channel capacity. Each entry is one sealed chunk awaiting witness
/// extraction. Bounded so a persistent extractor stall backpressures the
/// batch builder (via `send().await`) rather than blowing memory.
pub const CHUNK_WITNESS_CHANNEL_CAPACITY: usize = 64;

/// Initial backoff between retries when extraction fails.
const RETRY_INITIAL: Duration = Duration::from_millis(200);
/// Cap on a single retry sleep. Total retry duration is bounded by
/// [`RETRY_MAX_ATTEMPTS`], not by this cap on its own.
const RETRY_MAX_SLEEP: Duration = Duration::from_secs(5);
/// Hard cap on retry attempts before declaring permanent failure.
/// With exponential backoff capped at 5 s, 60 attempts ≈ 5 minutes —
/// long enough to absorb a slow exex without leaking the task forever.
const RETRY_MAX_ATTEMPTS: u32 = 60;

/// Sealed-chunk descriptor sent from the batch builder to the
/// chunk-witness task.
#[derive(Debug, Clone, Copy)]
pub struct ChunkExtractRequest {
    pub chunk_id: ChunkId,
    /// First block inside the chunk (NOT the chunk's `prev_block`
    /// ancestor — see [`alpen_ee_common::ChunkWitnessExtractFn`] docs).
    pub first_block: Hash,
    /// Last block inside the chunk.
    pub last_block: Hash,
}

/// Build the channel used between the batch builder and the
/// chunk-witness task.
pub fn chunk_witness_channel() -> (
    mpsc::Sender<ChunkExtractRequest>,
    mpsc::Receiver<ChunkExtractRequest>,
) {
    mpsc::channel(CHUNK_WITNESS_CHANNEL_CAPACITY)
}

/// Drains chunk-extract requests from `rx`, runs the extractor for each,
/// and persists the resulting witness. Returns when the sender side of
/// `rx` is dropped.
pub async fn chunk_witness_task(
    extractor: Arc<ChunkWitnessExtractFn>,
    store: Arc<dyn ChunkWitnessStore>,
    mut rx: mpsc::Receiver<ChunkExtractRequest>,
) {
    info!("chunk witness task started");
    while let Some(req) = rx.recv().await {
        let span = tracing::debug_span!("chunk_witness", chunk_id = ?req.chunk_id);
        process_request(&extractor, store.as_ref(), req)
            .instrument(span)
            .await;
    }
    warn!("chunk witness channel closed; exiting");
}

/// Runs the extractor for one chunk with bounded retry. Logged fields
/// shared across attempts (`chunk_id`) come from the caller's span;
/// per-attempt logs only need to add the changing `attempt` field.
#[instrument(level = "debug", skip_all)]
async fn process_request(
    extractor: &Arc<ChunkWitnessExtractFn>,
    store: &dyn ChunkWitnessStore,
    req: ChunkExtractRequest,
) {
    let ChunkExtractRequest {
        chunk_id,
        first_block,
        last_block,
    } = req;

    let mut sleep = RETRY_INITIAL;
    for attempt in 1..=RETRY_MAX_ATTEMPTS {
        let extractor_clone = Arc::clone(extractor);
        let join = task::spawn_blocking(move || (extractor_clone)(first_block, last_block)).await;

        match join {
            Ok(Ok(witness)) => {
                match store.put_chunk_witness(chunk_id, witness).await {
                    Ok(()) => debug!(attempt, "persisted chunk witness"),
                    Err(e) => error!(
                        attempt,
                        error = %e,
                        "failed to persist chunk witness; chunk will remain proof-blocked \
                         until a manual backfill writes the record"
                    ),
                }
                return;
            }
            Ok(Err(e)) => {
                if attempt < RETRY_MAX_ATTEMPTS {
                    debug!(
                        attempt,
                        sleep_ms = sleep.as_millis() as u64,
                        error = %e,
                        "chunk witness extraction failed; retrying (most likely the \
                         AccessedStateGenerator exex hasn't persisted records for one of the \
                         chunk's blocks yet)"
                    );
                    time::sleep(sleep).await;
                    sleep = (sleep * 2).min(RETRY_MAX_SLEEP);
                    continue;
                }
                error!(
                    attempt,
                    error = %e,
                    "chunk witness extraction exhausted retries; chunk will remain \
                     proof-blocked until manually re-extracted"
                );
                return;
            }
            Err(join_err) => {
                error!(
                    attempt,
                    error = %join_err,
                    "chunk witness extraction task panicked or was cancelled; giving up"
                );
                return;
            }
        }
    }
}

/// One-shot startup recovery: enqueue extraction requests for every
/// sealed chunk that has no persisted [`alpen_ee_common::ChunkWitnessRecord`].
///
/// Covers two cases the channel-only handoff can't:
/// - **Crash mid-extraction.** The chunk was sealed (so it's in [`BatchStorage`]) but the process
///   died before the background task wrote the witness — the `mpsc` request was lost with the
///   process.
/// - **Pre-existing chunks.** Chunks sealed before this PR (or otherwise ending up without a
///   witness row) would otherwise loop on `TransientFailure` until the prover retry budget exhausts
///   and the task becomes a permanent failure.
///
/// Returns the number of chunks enqueued. Each `send` is awaited so
/// channel backpressure naturally throttles a large backlog instead of
/// flooding the in-memory queue.
pub async fn backfill_missing_chunk_witnesses(
    batch_storage: &dyn BatchStorage,
    witness_store: &dyn ChunkWitnessStore,
    tx: &mpsc::Sender<ChunkExtractRequest>,
) -> Result<usize> {
    let Some((latest, _)) = batch_storage
        .get_latest_chunk()
        .await
        .map_err(|e| eyre!("get_latest_chunk: {e}"))?
    else {
        debug!("no chunks in storage; nothing to backfill");
        return Ok(0);
    };

    let latest_idx = latest.idx();
    let mut sent = 0usize;
    for idx in 0..=latest_idx {
        let Some((chunk, _)) = batch_storage
            .get_chunk_by_idx(idx)
            .await
            .map_err(|e| eyre!("get_chunk_by_idx({idx}): {e}"))?
        else {
            // Hole in the chunk index space (e.g. partial revert). Skip.
            continue;
        };
        let chunk_id = chunk.id();

        if witness_store
            .get_chunk_witness(chunk_id)
            .await
            .map_err(|e| eyre!("get_chunk_witness({chunk_id:?}): {e}"))?
            .is_some()
        {
            continue;
        }

        let Some(first_block) = chunk.blocks_iter().next() else {
            warn!(
                ?chunk_id,
                idx, "chunk has no blocks; skipping witness backfill"
            );
            continue;
        };

        let req = ChunkExtractRequest {
            chunk_id,
            first_block,
            last_block: chunk.last_block(),
        };
        tx.send(req)
            .await
            .map_err(|e| eyre!("send chunk extract request: {e}"))?;
        sent += 1;
        debug!(?chunk_id, idx, "enqueued chunk witness backfill");
    }

    if sent > 0 {
        info!(
            enqueued = sent,
            latest_idx, "chunk witness backfill complete"
        );
    } else {
        debug!(latest_idx, "no missing chunk witnesses; backfill skipped");
    }
    Ok(sent)
}
