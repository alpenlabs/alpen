use std::{sync::Arc, time::Duration};

use alpen_ee_common::{BatchStorage, ChunkProver, ChunkStatus, ChunkStorage};
use eyre::{eyre, Result};
use tokio::time;
use tracing::{error, warn};

use super::{
    ctx::ChunkLifecycleCtx,
    lifecycle::{try_advance_proof_pending, try_advance_sealed},
    state::ChunkProofCursor,
};

/// Polling interval for chunk proof lifecycle reconciliation.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Maximum chunks inspected per tick.
///
/// Bounds the per-tick query cost and acts as the de-facto in-flight cap on concurrent chunk
/// proofs: chunks past `floor + SCAN_WINDOW` are not submitted until the floor advances.
///
/// TODO(STR-3785): the dedicated proof scheduler should own the in-flight cap explicitly before
/// this window is widened or removed.
const SCAN_WINDOW: u64 = 64;

/// Runs the chunk proof lifecycle forever.
pub async fn chunk_lifecycle_task<P, S>(prover: Arc<P>, storage: Arc<S>)
where
    P: ChunkProver + Send + Sync + 'static,
    S: ChunkStorage + BatchStorage + 'static,
{
    let ctx = ChunkLifecycleCtx { prover, storage };

    // Recover the floor from batch status; fall back to a fresh cursor (advances forward on the
    // first tick) if recovery fails.
    let mut state = match ChunkProofCursor::recover(ctx.storage.as_ref()).await {
        Ok(state) => state,
        Err(e) => {
            error!(error = %e, "failed to recover chunk lifecycle state; starting from scratch");
            ChunkProofCursor::default()
        }
    };

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
/// Advances the [`ChunkProofCursor`] floor (skipping chunks of already-proven batches), then
/// within `[floor, floor + SCAN_WINDOW)` drives each chunk by its [`ChunkStatus`]: `Sealed` chunks
/// are submitted, `ProofPending` chunks are polled for completion/failure, and terminal chunks are
/// skipped. Per-chunk errors are isolated so one bad chunk does not starve the rest.
async fn process_cycle<P, S>(
    state: &mut ChunkProofCursor,
    ctx: &ChunkLifecycleCtx<P, S>,
) -> Result<()>
where
    P: ChunkProver,
    S: ChunkStorage + BatchStorage,
{
    let storage = ctx.storage.as_ref();
    let Some((latest_chunk, _)) = storage
        .get_latest_chunk()
        .await
        .map_err(|e| eyre!("get_latest_chunk: {e}"))?
    else {
        return Ok(());
    };
    let latest_idx = latest_chunk.idx();

    state.advance(storage, latest_idx).await?;
    let floor = state.floor();
    if floor > latest_idx {
        return Ok(());
    }
    let end = latest_idx.min(floor + SCAN_WINDOW - 1);

    for idx in floor..=end {
        let Some((chunk, status)) = storage
            .get_chunk_by_idx(idx)
            .await
            .map_err(|e| eyre!("get_chunk_by_idx({idx}): {e}"))?
        else {
            continue;
        };

        let result = match status {
            ChunkStatus::Sealed => try_advance_sealed(ctx, &chunk).await,
            ChunkStatus::ProofPending(_) => try_advance_proof_pending(ctx, &chunk).await,
            ChunkStatus::ProofReady(_) => Ok(()),
            ChunkStatus::ProofFailed(reason) => {
                warn!(
                    chunk_id = ?chunk.id(),
                    chunk_idx = chunk.idx(),
                    batch_idx = chunk.batch_idx(),
                    %reason,
                    "chunk proof permanently failed; lifecycle is blocked here until it is reset \
                     (e.g. via dbtool: clear the prover task and set the chunk back to Sealed)"
                );
                Ok(())
            }
        };
        if let Err(e) = result {
            warn!(
                chunk_idx = chunk.idx(),
                error = %e,
                "failed to advance chunk in proof lifecycle; continuing with next chunk"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use alpen_ee_common::{Chunk, ChunkId, InMemoryStorage, ProofGenerationStatus};
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
        Chunk::new(
            idx,
            test_hash(idx as u8),
            test_hash(idx as u8 + 1),
            idx + 1,
            0,
            vec![],
        )
    }

    /// With no batches recorded the floor is 0, so every sealed chunk is submitted, in index order.
    #[tokio::test]
    async fn submits_sealed_chunks_in_index_order() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        let chunk0 = make_chunk(0);
        let chunk1 = make_chunk(1);
        let chunk2 = make_chunk(2);
        storage.save_next_chunk(chunk0.clone()).await.unwrap();
        storage.save_next_chunk(chunk1.clone()).await.unwrap();
        storage.save_next_chunk(chunk2.clone()).await.unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        let ctx = ctx(prover.clone(), storage);
        process_cycle(&mut ChunkProofCursor::default(), &ctx)
            .await
            .unwrap();

        assert_eq!(prover.calls(), vec![chunk0.id(), chunk1.id(), chunk2.id()]);
    }

    #[tokio::test]
    async fn resubmits_pending_chunk_with_missing_task() {
        let storage = Arc::new(InMemoryStorage::new_empty());
        let chunk = make_chunk(0);
        storage.save_next_chunk(chunk.clone()).await.unwrap();
        storage
            .update_chunk_status(chunk.id(), ChunkStatus::ProofPending("lost".into()))
            .await
            .unwrap();

        let prover = Arc::new(RecordingChunkProver::default());
        prover.set_status(ProofGenerationStatus::NotStarted);
        let ctx = ctx(prover.clone(), storage);
        process_cycle(&mut ChunkProofCursor::default(), &ctx)
            .await
            .unwrap();

        assert_eq!(prover.calls(), vec![chunk.id()]);
    }

    #[tokio::test]
    async fn marks_permanent_failure_without_resubmit() {
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
        process_cycle(&mut ChunkProofCursor::default(), &ctx)
            .await
            .unwrap();

        assert!(prover.calls().is_empty());
        let (_chunk, status) = storage
            .get_chunk_by_id(chunk.id())
            .await
            .unwrap()
            .expect("chunk exists");
        assert!(matches!(status, ChunkStatus::ProofFailed(reason) if reason == "bad witness"));
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
        process_cycle(&mut ChunkProofCursor::default(), &ctx)
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
