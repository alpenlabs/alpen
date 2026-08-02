use std::{sync::Arc, time::Duration};

use alpen_ee_common::{BatchStorage, ChunkProver, ChunkStatus, ChunkStorage};
use eyre::{eyre, Result};
use tokio::time;
use tracing::{error, warn};

use super::{
    ctx::ChunkLifecycleCtx,
    lifecycle::{try_advance_proof_pending, try_advance_sealed},
    state::ChunkLifecycleState,
};

/// Polling interval for chunk proof lifecycle reconciliation.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Maximum chunks loaded from each storage-backed work queue per tick.
///
/// This bounds DB and prover-status work per tick. It is not a proof concurrency policy: sealed and
/// pending chunks are paged independently, so an early pending task cannot block later sealed
/// chunks from being submitted. It is also the signal for the end of a queue: a page shorter than
/// this rewinds its cursor to 0, so no row stays permanently above the cursor.
const WORK_QUERY_LIMIT: usize = 256;

/// Runs the chunk proof lifecycle forever.
pub async fn chunk_lifecycle_task<P, S>(prover: Arc<P>, storage: Arc<S>)
where
    P: ChunkProver + Send + Sync + 'static,
    S: ChunkStorage + BatchStorage + 'static,
{
    let ctx = ChunkLifecycleCtx { prover, storage };

    let mut state = ChunkLifecycleState::default();

    let mut poll_interval = time::interval(POLL_INTERVAL);

    loop {
        poll_interval.tick().await;

        if let Err(e) = process_cycle(&mut state, &ctx).await {
            error!(error = %e, "chunk proof lifecycle failed");
        }
    }
}

/// Reconcile chunk proofs for one tick.
///
/// Queries storage for sealed and proof-pending chunk work, then drives each chunk by status.
/// Sealed and pending chunks are paged independently so a slow or failed pending task does not
/// prevent later sealed chunks from being submitted. Per-chunk errors are isolated so one bad chunk
/// does not starve the rest, and a short page rewinds its cursor so a chunk left behind by a failed
/// submission is retried on the next tick rather than being skipped past forever.
async fn process_cycle<P, S>(
    state: &mut ChunkLifecycleState,
    ctx: &ChunkLifecycleCtx<P, S>,
) -> Result<()>
where
    P: ChunkProver,
    S: ChunkStorage + BatchStorage,
{
    let storage = ctx.storage.as_ref();

    let sealed_chunks = get_sealed_work_page(state, storage).await?;
    for (chunk, _status) in sealed_chunks {
        if let Err(e) = try_advance_sealed(ctx, &chunk).await {
            warn!(
                chunk_idx = chunk.idx(),
                error = %e,
                "failed to submit chunk proof; continuing with next sealed chunk"
            );
        }
    }

    let pending_chunks = get_pending_work_page(state, storage).await?;
    for (chunk, _status) in pending_chunks {
        if let Err(e) = try_advance_proof_pending(ctx, &chunk).await {
            warn!(
                chunk_idx = chunk.idx(),
                error = %e,
                "failed to poll chunk proof; continuing with next pending chunk"
            );
        }
    }

    Ok(())
}

async fn get_sealed_work_page<S>(
    state: &mut ChunkLifecycleState,
    storage: &S,
) -> Result<Vec<(alpen_ee_common::Chunk, ChunkStatus)>>
where
    S: ChunkStorage,
{
    let start_idx = state.sealed_poll_idx();
    let mut chunks = storage
        .get_sealed_chunks(start_idx, WORK_QUERY_LIMIT)
        .await
        .map_err(|e| eyre!("get_sealed_chunks({start_idx}): {e}"))?;
    // An empty page above the bottom means this tick would otherwise do nothing, so rescan
    // immediately instead of waiting a full poll interval.
    if chunks.is_empty() && start_idx != 0 {
        chunks = storage
            .get_sealed_chunks(0, WORK_QUERY_LIMIT)
            .await
            .map_err(|e| eyre!("get_sealed_chunks(0): {e}"))?;
    }

    state.record_sealed_page(
        chunks.last().map(|(chunk, _)| chunk.idx()),
        chunks.len() >= WORK_QUERY_LIMIT,
    );
    Ok(chunks)
}

async fn get_pending_work_page<S>(
    state: &mut ChunkLifecycleState,
    storage: &S,
) -> Result<Vec<(alpen_ee_common::Chunk, ChunkStatus)>>
where
    S: ChunkStorage,
{
    let start_idx = state.pending_poll_idx();
    let mut chunks = storage
        .get_proof_pending_chunks(start_idx, WORK_QUERY_LIMIT)
        .await
        .map_err(|e| eyre!("get_proof_pending_chunks({start_idx}): {e}"))?;
    // An empty page above the bottom means this tick would otherwise do nothing, so rescan
    // immediately instead of waiting a full poll interval.
    if chunks.is_empty() && start_idx != 0 {
        chunks = storage
            .get_proof_pending_chunks(0, WORK_QUERY_LIMIT)
            .await
            .map_err(|e| eyre!("get_proof_pending_chunks(0): {e}"))?;
    }

    state.record_pending_page(
        chunks.last().map(|(chunk, _)| chunk.idx()),
        chunks.len() >= WORK_QUERY_LIMIT,
    );
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };

    use alpen_ee_common::{Batch, Chunk, ChunkId, InMemoryStorage, ProofGenerationStatus};
    use async_trait::async_trait;
    use strata_acct_types::Hash;

    use super::*;
    use crate::test_utils::test_hash;

    #[derive(Debug)]
    struct RecordingChunkProver {
        calls: Mutex<Vec<ChunkId>>,
        status: Mutex<ProofGenerationStatus>,
        submit_failures: Mutex<HashSet<ChunkId>>,
    }

    impl Default for RecordingChunkProver {
        fn default() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                status: Mutex::new(ProofGenerationStatus::Pending),
                submit_failures: Mutex::new(HashSet::new()),
            }
        }
    }

    impl RecordingChunkProver {
        fn set_status(&self, status: ProofGenerationStatus) {
            *self.status.lock().unwrap() = status;
        }

        /// Make every submission of `chunk_id` fail, as a storage or paas error would.
        fn fail_submission_for(&self, chunk_id: ChunkId) {
            self.submit_failures.lock().unwrap().insert(chunk_id);
        }

        fn calls(&self) -> Vec<ChunkId> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChunkProver for RecordingChunkProver {
        async fn request_proof_generation(&self, chunk_id: ChunkId) -> eyre::Result<()> {
            self.calls.lock().unwrap().push(chunk_id);
            if self.submit_failures.lock().unwrap().contains(&chunk_id) {
                return Err(eyre!("submission failed"));
            }
            Ok(())
        }

        async fn check_proof_status(
            &self,
            _chunk_id: ChunkId,
        ) -> eyre::Result<ProofGenerationStatus> {
            Ok(self.status.lock().unwrap().clone())
        }
    }

    /// Build a ctx over an in-memory storage and recording prover for tests.
    fn ctx(
        prover: Arc<RecordingChunkProver>,
        storage: Arc<InMemoryStorage>,
    ) -> ChunkLifecycleCtx<RecordingChunkProver, InMemoryStorage> {
        ChunkLifecycleCtx { prover, storage }
    }

    fn make_chunk(idx: u64) -> Chunk {
        make_chunk_with_seed(idx, 0)
    }

    fn make_chunk_with_seed(idx: u64, seed: u8) -> Chunk {
        make_chunk_in_batch(idx, seed, 0)
    }

    fn make_chunk_in_batch(idx: u64, seed: u8, batch_idx: u64) -> Chunk {
        Chunk::new(
            idx,
            test_hash((idx as u8).wrapping_add(seed)),
            test_hash((idx as u8).wrapping_add(seed).wrapping_add(1)),
            idx + 1,
            batch_idx,
            vec![],
        )
    }

    fn wide_test_hash(value: u64) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        bytes[24..].copy_from_slice(&value.to_be_bytes());
        Hash::from(bytes)
    }

    fn make_wide_chunk(idx: u64) -> Chunk {
        Chunk::new(
            idx,
            wide_test_hash(idx.saturating_mul(2).saturating_add(1)),
            wide_test_hash(idx.saturating_mul(2).saturating_add(2)),
            idx + 1,
            0,
            vec![],
        )
    }

    async fn save_genesis_batch(storage: &InMemoryStorage) {
        let batch = Batch::new_genesis_batch(test_hash(250), 0).unwrap();
        storage.save_genesis_batch(batch).await.unwrap();
    }

    /// With a matching batch row present, every sealed chunk is submitted in index order.
    #[tokio::test]
    async fn submits_sealed_chunks_in_index_order() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        save_genesis_batch(&storage).await;
        let chunk0 = make_chunk(0);
        let chunk1 = make_chunk(1);
        let chunk2 = make_chunk(2);
        storage.save_next_chunk(chunk0.clone()).await.unwrap();
        storage.save_next_chunk(chunk1.clone()).await.unwrap();
        storage.save_next_chunk(chunk2.clone()).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage);
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

        assert_eq!(prover.calls(), vec![chunk0.id(), chunk1.id(), chunk2.id()]);
    }

    /// A chunk whose batch row was removed by a reorg must not be proven.
    #[tokio::test]
    async fn skips_sealed_chunk_whose_batch_was_reverted() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        // Storage tip is batch 1, while the chunk's batch 0 row is gone.
        let batch = Batch::new(1, test_hash(200), test_hash(201), 1, vec![]).unwrap();
        storage.save_next_batch(batch).await.unwrap();
        let chunk = make_chunk_in_batch(0, 0, 0);
        storage.save_next_chunk(chunk).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage);
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

        assert!(prover.calls().is_empty());
    }

    /// Chunks of the batch still being accumulated have no batch row yet. That is indistinguishable
    /// from a reverted batch, so they are deferred until the owning batch seals.
    #[tokio::test]
    async fn defers_sealed_chunk_of_still_accumulating_batch() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        save_genesis_batch(&storage).await;
        // Batch 1 is still accumulating, so only the genesis batch row exists.
        let chunk = make_chunk_in_batch(0, 0, 1);
        storage.save_next_chunk(chunk.clone()).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage);
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

        assert!(prover.calls().is_empty());
    }

    /// With no batches in storage at all every chunk's batch row is absent, so nothing is proven.
    #[tokio::test]
    async fn defers_sealed_chunk_when_no_batches_exist() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        let chunk = make_chunk(0);
        storage.save_next_chunk(chunk.clone()).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage);
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

        assert!(prover.calls().is_empty());
    }

    /// A deferred chunk is picked up on a later tick, once its owning batch seals and writes its
    /// row. Proving therefore starts at batch seal, without waiting for L1 data availability.
    #[tokio::test]
    async fn submits_deferred_chunk_once_its_batch_seals() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        save_genesis_batch(&storage).await;
        let chunk = make_chunk_in_batch(0, 0, 1);
        storage.save_next_chunk(chunk.clone()).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage.clone());
        let mut state = ChunkLifecycleState::default();
        process_cycle(&mut state, &ctx).await.unwrap();
        assert!(prover.calls().is_empty());

        // Batch 1 seals: its row lands in storage, still without any L1 DA.
        let batch = Batch::new(1, test_hash(200), test_hash(201), 1, vec![]).unwrap();
        storage.save_next_batch(batch).await.unwrap();

        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(prover.calls(), vec![chunk.id()]);
    }

    #[tokio::test]
    async fn pending_page_does_not_block_later_sealed_chunks() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        save_genesis_batch(&storage).await;
        for idx in 0..WORK_QUERY_LIMIT as u64 {
            let chunk = make_chunk(idx);
            storage.save_next_chunk(chunk.clone()).await.unwrap();
            storage
                .update_chunk_status(chunk.id(), ChunkStatus::ProofPending("task".into()))
                .await
                .unwrap();
        }
        let sealed = make_chunk(WORK_QUERY_LIMIT as u64);
        storage.save_next_chunk(sealed.clone()).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage);
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

        assert_eq!(prover.calls(), vec![sealed.id()]);
    }

    #[tokio::test]
    async fn sealed_cursor_advances_past_a_stuck_first_page() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        save_genesis_batch(&storage).await;
        for idx in 0..=WORK_QUERY_LIMIT as u64 {
            storage.save_next_chunk(make_wide_chunk(idx)).await.unwrap();
        }

        // The recording prover intentionally leaves every chunk Sealed. The
        // second cycle must nevertheless advance past the first full page.
        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage);
        let mut state = ChunkLifecycleState::default();
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(prover.calls().len(), WORK_QUERY_LIMIT);

        process_cycle(&mut state, &ctx).await.unwrap();
        let calls = prover.calls();
        assert_eq!(calls.len(), WORK_QUERY_LIMIT + 1);
        assert_eq!(
            calls.last(),
            Some(&make_wide_chunk(WORK_QUERY_LIMIT as u64).id())
        );
    }

    #[tokio::test]
    async fn sealed_cursor_wraps_to_reorged_lower_chunks() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        save_genesis_batch(&storage).await;
        let chunk0 = make_chunk(0);
        storage.save_next_chunk(chunk0.clone()).await.unwrap();
        storage
            .update_chunk_status(chunk0.id(), ChunkStatus::ProofPending("task".into()))
            .await
            .unwrap();
        for idx in 1..=2 {
            storage.save_next_chunk(make_chunk(idx)).await.unwrap();
        }
        storage.revert_chunks_from(1).await.unwrap();
        let chunk1 = make_chunk_with_seed(1, 10);
        let chunk2 = make_chunk_with_seed(2, 10);
        storage.save_next_chunk(chunk1.clone()).await.unwrap();
        storage.save_next_chunk(chunk2.clone()).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage);
        let mut state = ChunkLifecycleState::default();
        state.record_sealed_page(Some(2), true);
        process_cycle(&mut state, &ctx).await.unwrap();

        assert_eq!(prover.calls(), vec![chunk1.id(), chunk2.id()]);
    }

    /// A chunk whose submission errored stays `Sealed` and must be retried on the next tick, even
    /// though higher-index chunks keep the sealed work pages non-empty.
    #[tokio::test]
    async fn retries_errored_sealed_chunk_on_next_tick() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        save_genesis_batch(&storage).await;
        let chunk0 = make_chunk(0);
        let chunk1 = make_chunk(1);
        storage.save_next_chunk(chunk0.clone()).await.unwrap();
        storage.save_next_chunk(chunk1.clone()).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        prover.fail_submission_for(chunk0.id());
        let ctx = ctx(prover.clone(), storage.clone());
        let mut state = ChunkLifecycleState::default();
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(prover.calls(), vec![chunk0.id(), chunk1.id()]);

        // Chunk 1 was accepted and left the sealed queue; chunk 0's error left it behind. A newly
        // sealed chunk 2 keeps the queue non-empty above chunk 0.
        storage
            .update_chunk_status(chunk1.id(), ChunkStatus::ProofPending("task".into()))
            .await
            .unwrap();
        let chunk2 = make_chunk(2);
        storage.save_next_chunk(chunk2.clone()).await.unwrap();

        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(
            prover.calls(),
            vec![chunk0.id(), chunk1.id(), chunk0.id(), chunk2.id()]
        );
    }

    #[tokio::test]
    async fn resubmits_pending_chunk_with_missing_task() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        save_genesis_batch(&storage).await;
        let chunk = make_chunk(0);
        storage.save_next_chunk(chunk.clone()).await.unwrap();
        storage
            .update_chunk_status(chunk.id(), ChunkStatus::ProofPending("lost".into()))
            .await
            .unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        prover.set_status(ProofGenerationStatus::NotStarted);
        let ctx = ctx(prover.clone(), storage);
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

        assert_eq!(prover.calls(), vec![chunk.id()]);
    }

    #[tokio::test]
    async fn leaves_permanent_failure_pending_without_resubmit() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        let chunk = make_chunk(0);
        storage.save_next_chunk(chunk.clone()).await.unwrap();
        storage
            .update_chunk_status(chunk.id(), ChunkStatus::ProofPending("task".into()))
            .await
            .unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        prover.set_status(ProofGenerationStatus::Failed {
            reason: "bad witness".into(),
        });
        let ctx = ctx(prover.clone(), storage.clone());
        let mut state = ChunkLifecycleState::default();
        process_cycle(&mut state, &ctx).await.unwrap();

        // Repeated observations neither resubmit nor mutate the durable chunk status.
        process_cycle(&mut state, &ctx).await.unwrap();

        assert!(prover.calls().is_empty());
        let (_chunk, status) = storage
            .get_chunk_by_id(chunk.id())
            .await
            .unwrap()
            .expect("chunk exists");
        assert!(matches!(status, ChunkStatus::ProofPending(task) if task == "task"));
    }

    #[tokio::test]
    async fn records_completed_pending_chunk_status() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        let chunk = make_chunk(0);
        storage.save_next_chunk(chunk.clone()).await.unwrap();
        storage
            .update_chunk_status(chunk.id(), ChunkStatus::ProofPending("task".into()))
            .await
            .unwrap();

        let proof_id = test_hash(7);
        let prover = Arc::new(RecordingChunkProver::default());
        prover.set_status(ProofGenerationStatus::Ready { proof_id });
        let ctx = ctx(prover, storage.clone());
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

        let (_chunk, status) = storage
            .get_chunk_by_id(chunk.id())
            .await
            .unwrap()
            .expect("chunk exists");
        assert!(matches!(status, ChunkStatus::ProofReady(id) if id == proof_id));
    }
}
