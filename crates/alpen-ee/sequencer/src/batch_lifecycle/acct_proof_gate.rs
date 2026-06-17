use alpen_ee_common::{Batch, ChunkId, ChunkStatus, ChunkStorage};
use eyre::{bail, Result};

/// Decision for whether an acct proof can be requested for a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AcctProofGateDecision {
    /// Every linked chunk proof is ready.
    Ready,
    /// The batch-to-chunk link has not been written yet.
    WaitingForChunkLinks,
    /// At least one linked chunk proof is still in progress or not submitted.
    WaitingForChunkProof {
        chunk_id: ChunkId,
        status: ChunkStatus,
    },
    /// A linked chunk is not present in storage.
    ///
    /// Transient during a reorg (a chunk revert races batch-chunk linkage repair); a persisting
    /// occurrence indicates storage corruption.
    WaitingForMissingChunk { chunk_id: ChunkId },
    /// A linked chunk proof reached a terminal failed state.
    BlockedByChunkFailure { chunk_id: ChunkId, reason: String },
}

/// Checks whether the acct proof request has all chunk proof inputs available.
///
/// This is a read-model decision, not a persisted state. Missing links and missing referenced
/// chunks can be transient during a reorg (a chunk revert races batch-chunk linkage repair), so
/// they return waiting decisions. An empty non-genesis chunk list is a true storage invariant
/// violation and returns an error.
pub(crate) async fn check_acct_proof_gate(
    batch: &Batch,
    chunk_storage: &impl ChunkStorage,
) -> Result<AcctProofGateDecision> {
    let batch_id = batch.id();
    let Some(chunk_ids) = chunk_storage.get_batch_chunks(batch_id).await? else {
        return Ok(AcctProofGateDecision::WaitingForChunkLinks);
    };

    if chunk_ids.is_empty() {
        if batch.idx() == 0 {
            return Ok(AcctProofGateDecision::Ready);
        }

        bail!("non-genesis batch {batch_id} has an empty chunk list");
    }

    for chunk_id in chunk_ids {
        let Some((_chunk, status)) = chunk_storage.get_chunk_by_id(chunk_id).await? else {
            return Ok(AcctProofGateDecision::WaitingForMissingChunk { chunk_id });
        };

        match status {
            ChunkStatus::ProofReady(_) => {}
            ChunkStatus::ProofFailed(reason) => {
                return Ok(AcctProofGateDecision::BlockedByChunkFailure { chunk_id, reason });
            }
            status => {
                return Ok(AcctProofGateDecision::WaitingForChunkProof { chunk_id, status });
            }
        }
    }

    Ok(AcctProofGateDecision::Ready)
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{BatchStorage, Chunk, ChunkId, ChunkStorage, InMemoryStorage};

    use super::*;
    use crate::batch_lifecycle::test_utils::{make_batch, make_genesis_batch, test_hash};

    async fn storage_with_batch() -> (InMemoryStorage, Batch) {
        let storage = InMemoryStorage::new_empty();
        let genesis = make_genesis_batch(0);
        let batch = make_batch(1, 0, 1);
        storage.save_genesis_batch(genesis).await.unwrap();
        storage.save_next_batch(batch.clone()).await.unwrap();
        (storage, batch)
    }

    fn chunk_for_batch(batch: &Batch) -> Chunk {
        Chunk::new(
            0,
            batch.prev_block(),
            batch.last_block(),
            batch.last_blocknum(),
            batch.idx(),
            vec![],
        )
    }

    #[tokio::test]
    async fn waits_when_batch_chunks_are_not_linked() {
        let (storage, batch) = storage_with_batch().await;

        let decision = check_acct_proof_gate(&batch, &storage).await.unwrap();

        assert_eq!(decision, AcctProofGateDecision::WaitingForChunkLinks);
    }

    #[tokio::test]
    async fn waits_when_batch_links_missing_chunk() {
        let (storage, batch) = storage_with_batch().await;
        let missing_chunk_id = ChunkId::from_parts(test_hash(10), test_hash(11));
        storage
            .set_batch_chunks(batch.id(), vec![missing_chunk_id])
            .await
            .unwrap();

        let decision = check_acct_proof_gate(&batch, &storage).await.unwrap();

        assert_eq!(
            decision,
            AcctProofGateDecision::WaitingForMissingChunk {
                chunk_id: missing_chunk_id
            }
        );
    }

    #[tokio::test]
    async fn errors_when_non_genesis_batch_has_empty_chunk_list() {
        let (storage, batch) = storage_with_batch().await;
        storage.set_batch_chunks(batch.id(), vec![]).await.unwrap();

        let err = check_acct_proof_gate(&batch, &storage).await.unwrap_err();

        assert!(
            err.to_string().contains("empty chunk list"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn blocks_when_linked_chunk_failed() {
        let (storage, batch) = storage_with_batch().await;
        let chunk = chunk_for_batch(&batch);
        let chunk_id = chunk.id();
        storage.save_next_chunk(chunk).await.unwrap();
        storage
            .set_batch_chunks(batch.id(), vec![chunk_id])
            .await
            .unwrap();
        storage
            .update_chunk_status(chunk_id, ChunkStatus::ProofFailed("bad witness".into()))
            .await
            .unwrap();

        let decision = check_acct_proof_gate(&batch, &storage).await.unwrap();

        assert_eq!(
            decision,
            AcctProofGateDecision::BlockedByChunkFailure {
                chunk_id,
                reason: "bad witness".into(),
            }
        );
    }
}
