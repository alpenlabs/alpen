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
/// chunks from being submitted.
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
/// does not starve the rest.
async fn process_cycle<P, S>(
    state: &mut ChunkLifecycleState,
    ctx: &ChunkLifecycleCtx<P, S>,
) -> Result<()>
where
    P: ChunkProver,
    S: ChunkStorage + BatchStorage,
{
    let storage = ctx.storage.as_ref();

    let sealed_chunks = get_sealed_work_page(storage).await?;
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

async fn get_sealed_work_page<S>(storage: &S) -> Result<Vec<(alpen_ee_common::Chunk, ChunkStatus)>>
where
    S: ChunkStorage,
{
    storage
        .get_sealed_chunks(0, WORK_QUERY_LIMIT)
        .await
        .map_err(|e| eyre!("get_sealed_chunks(0): {e}"))
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
    if chunks.is_empty() && start_idx != 0 {
        state.wrap_pending_poll_idx();
        chunks = storage
            .get_proof_pending_chunks(0, WORK_QUERY_LIMIT)
            .await
            .map_err(|e| eyre!("get_proof_pending_chunks(0): {e}"))?;
    }

    state.advance_pending_poll_idx(chunks.last().map(|(chunk, _)| chunk.idx()));
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use alpen_ee_common::{Batch, Chunk, ChunkId, InMemoryStorage, ProofGenerationStatus};
    use async_trait::async_trait;

    use super::*;
    use crate::test_utils::test_hash;

    #[derive(Debug)]
    struct RecordingChunkProver {
        calls: Mutex<Vec<ChunkId>>,
        status: Mutex<ProofGenerationStatus>,
    }

    impl Default for RecordingChunkProver {
        fn default() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                status: Mutex::new(ProofGenerationStatus::Pending),
            }
        }
    }

    impl RecordingChunkProver {
        fn set_status(&self, status: ProofGenerationStatus) {
            *self.status.lock().unwrap() = status;
        }

        fn calls(&self) -> Vec<ChunkId> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChunkProver for RecordingChunkProver {
        async fn request_proof_generation(&self, chunk_id: ChunkId) -> eyre::Result<()> {
            self.calls.lock().unwrap().push(chunk_id);
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
        Chunk::new(
            idx,
            test_hash((idx as u8).wrapping_add(seed)),
            test_hash((idx as u8).wrapping_add(seed).wrapping_add(1)),
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

    #[tokio::test]
    async fn skips_sealed_chunk_whose_batch_was_reverted() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        let chunk = make_chunk(0);
        storage.save_next_chunk(chunk).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage);
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

        assert!(prover.calls().is_empty());
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
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

        assert_eq!(prover.calls(), vec![chunk1.id(), chunk2.id()]);
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
        process_cycle(&mut ChunkLifecycleState::default(), &ctx)
            .await
            .unwrap();

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
