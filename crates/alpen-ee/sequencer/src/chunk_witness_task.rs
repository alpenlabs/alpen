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

use alpen_ee_common::{ChunkId, ChunkWitnessExtractFn, ChunkWitnessStore};
use strata_acct_types::Hash;
use tokio::{sync::mpsc, task, time};
use tracing::{debug, error, info, warn};

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
        process_request(&extractor, store.as_ref(), req).await;
    }
    warn!("chunk witness channel closed; exiting");
}

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
                    Ok(()) => {
                        debug!(?chunk_id, attempt, "persisted chunk witness");
                    }
                    Err(e) => {
                        error!(
                            ?chunk_id,
                            attempt,
                            error = %e,
                            "failed to persist chunk witness; chunk will remain proof-blocked \
                             until a manual backfill writes the record"
                        );
                    }
                }
                return;
            }
            Ok(Err(e)) => {
                if attempt < RETRY_MAX_ATTEMPTS {
                    debug!(
                        ?chunk_id,
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
                    ?chunk_id,
                    attempt,
                    error = %e,
                    "chunk witness extraction exhausted retries; chunk will remain \
                     proof-blocked until manually re-extracted"
                );
                return;
            }
            Err(join_err) => {
                error!(
                    ?chunk_id,
                    attempt,
                    error = %join_err,
                    "chunk witness extraction task panicked or was cancelled; giving up"
                );
                return;
            }
        }
    }
}
