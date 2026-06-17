//! Progress tracking for the chunk proof lifecycle.

use alpen_ee_common::{Batch, BatchStatus, BatchStorage, ChunkStatus, ChunkStorage};
use eyre::{eyre, Result};

/// Tracks the chunk proof lifecycle's progress through a single cursor, the *floor*: the first
/// chunk index that is not yet `ProofReady` and so might still need proof work.
///
/// Everything below the floor is `ProofReady`. The floor advances only past `ProofReady` chunks, so
/// it pins on the first `Sealed`/`ProofPending` chunk (the in-flight edge) or on a `ProofFailed`
/// chunk. Pinning on failure is deliberate: it keeps the failed chunk inside the working window so
/// the lifecycle keeps surfacing it and a manual reset (e.g. via dbtool) is picked up rather than
/// skipped. On startup or after a reorg the floor is seeded from batch status — a batch only
/// reaches `ProofPending` once all of its chunks are proven, so a proven batch's chunks are a cheap
/// lower bound — and then refined forward.
#[derive(Debug, Default)]
pub(super) struct ChunkProofCursor {
    floor: u64,
}

impl ChunkProofCursor {
    /// Recover the floor from batch status, e.g. on startup.
    pub(super) async fn recover(storage: &(impl ChunkStorage + BatchStorage)) -> Result<Self> {
        Ok(Self {
            floor: derive_floor(storage).await?,
        })
    }

    /// The first chunk index that might still need proof work.
    pub(super) fn floor(&self) -> u64 {
        self.floor
    }

    /// Move the floor forward past `ProofReady` chunks, so finished chunks are not re-scanned.
    /// Advancing per chunk (rather than per batch) keeps the cap purely on in-flight proofs and
    /// never bounds how many chunks a batch may have. It stops at the first chunk that is not
    /// `ProofReady` — `Sealed`/`ProofPending` (still in flight) or `ProofFailed` (kept in the
    /// window so it stays visible and a manual reset is picked up). Rebuilds the floor from
    /// batch status if a reorg dropped the chunk tip below it.
    pub(super) async fn advance(
        &mut self,
        storage: &(impl ChunkStorage + BatchStorage),
        latest_chunk_idx: u64,
    ) -> Result<()> {
        if self.floor > latest_chunk_idx + 1 {
            self.floor = derive_floor(storage).await?;
            return Ok(());
        }

        while self.floor <= latest_chunk_idx {
            let Some((_, status)) = storage
                .get_chunk_by_idx(self.floor)
                .await
                .map_err(|e| eyre!("get_chunk_by_idx({}): {e}", self.floor))?
            else {
                break;
            };
            if matches!(status, ChunkStatus::ProofReady(_)) {
                self.floor += 1;
            } else {
                break;
            }
        }

        Ok(())
    }
}

/// Whether a batch has reached `ProofPending` or beyond, which implies all of its chunks are
/// proven.
fn batch_proven(status: &BatchStatus) -> bool {
    matches!(
        status,
        BatchStatus::ProofPending { .. } | BatchStatus::ProofReady { .. }
    )
}

/// Compute the floor: the first chunk index that might still need proof work.
///
/// Everything below the floor is already proven, because a batch reaches `ProofPending` only once
/// all of its chunks are proven. So the floor is just past the last chunk of the most recently
/// proven batch, or 0 if no batch is proven yet.
async fn derive_floor(storage: &(impl ChunkStorage + BatchStorage)) -> Result<u64> {
    let Some(batch) = latest_proven_batch(storage).await? else {
        return Ok(0);
    };
    let last_chunk = batch_last_chunk_idx(storage, &batch).await?;
    Ok(last_chunk.map_or(0, |idx| idx + 1))
}

/// The most recently proven batch (status `ProofPending` or beyond), or `None` if none is proven.
///
/// Batches reach `ProofPending` strictly in order, so a backward scan from the tip stops at the
/// first proven batch.
async fn latest_proven_batch(
    storage: &(impl ChunkStorage + BatchStorage),
) -> Result<Option<Batch>> {
    let Some((latest, _)) = storage
        .get_latest_batch()
        .await
        .map_err(|e| eyre!("get_latest_batch: {e}"))?
    else {
        return Ok(None);
    };

    for batch_idx in (0..=latest.idx()).rev() {
        let Some((batch, status)) = storage
            .get_batch_by_idx(batch_idx)
            .await
            .map_err(|e| eyre!("get_batch_by_idx({batch_idx}): {e}"))?
        else {
            continue;
        };
        if batch_proven(&status) {
            return Ok(Some(batch));
        }
    }

    Ok(None)
}

/// The highest chunk index belonging to `batch`, or `None` if it has no chunks (e.g. genesis).
async fn batch_last_chunk_idx(
    storage: &(impl ChunkStorage + BatchStorage),
    batch: &Batch,
) -> Result<Option<u64>> {
    let Some(chunk_ids) = storage
        .get_batch_chunks(batch.id())
        .await
        .map_err(|e| eyre!("get_batch_chunks: {e}"))?
    else {
        return Ok(None);
    };

    let mut last = None;
    for chunk_id in chunk_ids {
        if let Some((chunk, _)) = storage
            .get_chunk_by_id(chunk_id)
            .await
            .map_err(|e| eyre!("get_chunk_by_id: {e}"))?
        {
            last = Some(last.map_or(chunk.idx(), |l: u64| l.max(chunk.idx())));
        }
    }

    Ok(last)
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{BatchStatus, BatchStorage, Chunk, InMemoryStorage};

    use super::*;
    use crate::{
        batch_lifecycle::test_utils::{make_batch, make_genesis_batch},
        test_utils::test_hash,
    };

    fn chunk(idx: u64, batch_idx: u64) -> Chunk {
        Chunk::new(
            idx,
            test_hash(idx as u8),
            test_hash(idx as u8 + 1),
            idx + 1,
            batch_idx,
            vec![],
        )
    }

    /// The floor is one past the last chunk of the latest `ProofPending` batch.
    #[tokio::test]
    async fn recover_floor_past_latest_proven_batch() {
        let storage = InMemoryStorage::new_empty();
        storage
            .save_genesis_batch(make_genesis_batch(0))
            .await
            .unwrap();

        // batch 1 is proven (ProofPending) and owns chunks 0 and 1.
        let batch1 = make_batch(1, 0, 1);
        storage.save_next_batch(batch1.clone()).await.unwrap();
        // batch 2 is still sealed and owns chunk 2.
        let batch2 = make_batch(2, 1, 2);
        storage.save_next_batch(batch2.clone()).await.unwrap();

        for c in [chunk(0, 1), chunk(1, 1), chunk(2, 2)] {
            storage.save_next_chunk(c).await.unwrap();
        }
        storage
            .set_batch_chunks(batch1.id(), vec![chunk(0, 1).id(), chunk(1, 1).id()])
            .await
            .unwrap();
        storage
            .update_batch_status(batch1.id(), BatchStatus::ProofPending { da: vec![] })
            .await
            .unwrap();

        let state = ChunkProofCursor::recover(&storage).await.unwrap();
        assert_eq!(state.floor(), 2);
    }

    /// `advance` steps the floor forward over `ProofReady` chunks one at a time, regardless of
    /// batch boundaries, and pins on a `ProofFailed` chunk (rather than skipping it).
    #[tokio::test]
    async fn advance_steps_over_proven_chunks_and_pins_on_failure() {
        let storage = InMemoryStorage::new_empty();
        for c in [chunk(0, 1), chunk(1, 1), chunk(2, 1)] {
            storage.save_next_chunk(c).await.unwrap();
        }

        let mut state = ChunkProofCursor::default();

        // all Sealed: floor stays at 0.
        state.advance(&storage, 2).await.unwrap();
        assert_eq!(state.floor(), 0);

        // chunk 0 proven: floor steps to 1; chunk 1 still Sealed, so it stops.
        storage
            .update_chunk_status(chunk(0, 1).id(), ChunkStatus::ProofReady(test_hash(9)))
            .await
            .unwrap();
        state.advance(&storage, 2).await.unwrap();
        assert_eq!(state.floor(), 1);

        // chunk 1 permanently failed: the floor pins there (does not skip past the failure).
        storage
            .update_chunk_status(chunk(1, 1).id(), ChunkStatus::ProofFailed("boom".into()))
            .await
            .unwrap();
        state.advance(&storage, 2).await.unwrap();
        assert_eq!(state.floor(), 1);
    }

    /// A reverted chunk tip below the cached floor triggers a rebuild.
    #[tokio::test]
    async fn advance_rebuilds_when_tip_reverts_below_floor() {
        let storage = InMemoryStorage::new_empty();
        // Pretend the floor advanced to 5 in a previous life, then everything was reverted away.
        let mut state = ChunkProofCursor { floor: 5 };
        state.advance(&storage, 0).await.unwrap();
        assert_eq!(state.floor(), 0);
    }
}
