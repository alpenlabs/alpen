//! Reconciles local EE batch/chunk artifacts against OL-accepted state.
//!
//! After an EE predicate rotation (the snark account `update_vk` changes), any
//! batch the OL has not yet accepted was proven under the old ELF and will be
//! rejected by the OL STF on resubmit. The prover is idempotent on
//! `ProofReady`, so it never re-proves a batch that already has a proof. On
//! startup we revert every batch past the OL-accepted `seq_no` (and the chunks
//! belonging to those batches), so the batch and chunk builders rebuild and
//! re-prove them under the new ELF. This is the EE-side analog of the OL
//! checkpoint reconciler (PR #1926).
//!
//! Treats the OL account `seq_no` as ground truth, the same way the OL
//! reconciler treats the ASM verified tip. Only artifacts past the accepted
//! point are touched; accepted history is left alone.

use alpen_ee_common::{BatchStorage, ChunkStorage, SequencerOLClient};
use eyre::{Context, Result};
use tracing::{info, warn};

/// Reverts unaccepted EE batches and their chunks so they re-prove under the
/// new ELF. Must run before the batch/chunk builders initialize.
pub(crate) async fn reconcile_unaccepted_ee_artifacts<S, OL>(
    storage: &S,
    ol_client: &OL,
) -> Result<()>
where
    S: BatchStorage + ChunkStorage,
    OL: SequencerOLClient + ?Sized,
{
    let account_state = ol_client
        .get_latest_account_state()
        .await
        .context("fetch latest OL account state for EE reconciliation")?;
    let accepted_seq_no = *account_state.seq_no.inner();

    // Compute the chunk revert boundary from the first unaccepted batch before
    // reverting batches, since `revert_batches` drops the batch->chunk linkage.
    let chunk_revert_from = chunk_revert_boundary(storage, accepted_seq_no).await?;

    if let Some(from_idx) = chunk_revert_from {
        storage
            .revert_chunks_from(from_idx)
            .await
            .context("revert unaccepted EE chunks")?;
    }

    storage
        .revert_batches(accepted_seq_no)
        .await
        .context("revert unaccepted EE batches")?;

    info!(
        accepted_seq_no,
        chunk_revert_from = ?chunk_revert_from,
        "reconciled unaccepted EE batch/chunk artifacts against OL accepted seq_no"
    );

    Ok(())
}

/// Returns the first chunk idx belonging to the first unaccepted batch, which is
/// the boundary to revert chunks from. Returns `None` when there is nothing past
/// the accepted point, or when the batch->chunk linkage is not yet persisted.
async fn chunk_revert_boundary<S>(storage: &S, accepted_seq_no: u64) -> Result<Option<u64>>
where
    S: BatchStorage + ChunkStorage,
{
    let Some(first_unaccepted_batch_idx) = accepted_seq_no.checked_add(1) else {
        return Ok(None);
    };

    let Some((batch, _)) = storage
        .get_batch_by_idx(first_unaccepted_batch_idx)
        .await
        .context("get first unaccepted batch")?
    else {
        // Nothing built past the accepted point.
        return Ok(None);
    };

    let Some(chunk_ids) = storage
        .get_batch_chunks(batch.id())
        .await
        .context("get chunks of first unaccepted batch")?
    else {
        // Linkage not persisted yet (batch still pre-DA). Leave chunks for the
        // chunk-builder recovery to handle, and flag it: a rotated chunk ELF
        // would leave stale chunk proofs here. Verify on the rehearsal.
        warn!(
            first_unaccepted_batch_idx,
            "no chunk linkage for first unaccepted batch; skipping chunk revert"
        );
        return Ok(None);
    };

    let mut from_idx: Option<u64> = None;
    for chunk_id in &chunk_ids {
        if let Some((chunk, _)) = storage
            .get_chunk_by_id(*chunk_id)
            .await
            .context("resolve chunk idx for revert boundary")?
        {
            from_idx = Some(from_idx.map_or(chunk.idx(), |cur| cur.min(chunk.idx())));
        }
    }

    Ok(from_idx)
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{
        Batch, BatchStorage, Chunk, ChunkStorage, InMemoryStorage, MockSequencerOLClient,
        OLAccountStateView,
    };
    use strata_acct_types::Hash;
    use strata_snark_acct_types::{ProofState, Seqno};

    use super::reconcile_unaccepted_ee_artifacts;

    /// Builds a distinct, non-zero [`Hash`] keyed by `tag`.
    fn hash(tag: u64) -> Hash {
        let mut bytes = [0u8; 32];
        // Keep byte 0 set so the hash is non-zero even when `tag` is 0, and
        // encode `tag` in the tail so distinct tags yield distinct hashes.
        bytes[0] = 1;
        bytes[24..32].copy_from_slice(&tag.to_le_bytes());
        Hash::from(bytes)
    }

    /// Genesis batch lives at idx 0 and is never an accepted update.
    fn genesis_batch() -> Batch {
        Batch::new_genesis_batch(hash(1), 0).expect("genesis batch")
    }

    /// Non-genesis batch at `idx` with a unique, non-empty block range.
    fn batch(idx: u64) -> Batch {
        Batch::new(idx, hash(2 * idx), hash(2 * idx + 1), idx * 10, Vec::new())
            .expect("non-genesis batch")
    }

    /// Chunk at `idx` owned by `batch_idx`, with a unique block range. The id is
    /// derived from the block range only, so it is stable across calls.
    fn chunk(idx: u64, batch_idx: u64) -> Chunk {
        Chunk::new(
            idx,
            hash(1000 + 2 * idx),
            hash(1001 + 2 * idx),
            idx * 10,
            batch_idx,
            Vec::new(),
        )
    }

    /// Mock OL client whose latest account state reports `seq_no` (the highest
    /// accepted batch idx; the first unaccepted batch is `seq_no + 1`).
    fn ol_client_at_seq_no(seq_no: u64) -> MockSequencerOLClient {
        let mut ol_client = MockSequencerOLClient::new();
        ol_client
            .expect_get_latest_account_state()
            .returning(move || {
                Ok(OLAccountStateView {
                    seq_no: Seqno::new(seq_no),
                    proof_state: ProofState::new(Hash::zero(), 0),
                })
            });
        ol_client
    }

    async fn has_batch(storage: &InMemoryStorage, idx: u64) -> bool {
        storage.get_batch_by_idx(idx).await.unwrap().is_some()
    }

    async fn has_chunk(storage: &InMemoryStorage, idx: u64) -> bool {
        storage.get_chunk_by_idx(idx).await.unwrap().is_some()
    }

    /// Reverts batches past the accepted seq_no and the chunks of the first
    /// unaccepted batch, while leaving accepted batches and their chunks intact.
    #[tokio::test]
    async fn reverts_unaccepted_batches_and_their_chunks() {
        let storage = InMemoryStorage::new_empty();
        storage.save_genesis_batch(genesis_batch()).await.unwrap();
        for idx in 1..=3 {
            storage.save_next_batch(batch(idx)).await.unwrap();
        }
        // Chunks 0,1 belong to accepted batch 1; chunks 2,3 to the first
        // unaccepted batch (idx 2).
        for idx in 0..=3 {
            let owner = if idx < 2 { 1 } else { 2 };
            storage.save_next_chunk(chunk(idx, owner)).await.unwrap();
        }
        storage
            .set_batch_chunks(batch(2).id(), vec![chunk(2, 2).id(), chunk(3, 2).id()])
            .await
            .unwrap();

        // seq_no = 1 => accepted batch idxs 0,1; first unaccepted = 2.
        let ol_client = ol_client_at_seq_no(1);
        reconcile_unaccepted_ee_artifacts(&storage, &ol_client)
            .await
            .unwrap();

        assert!(has_batch(&storage, 0).await, "genesis retained");
        assert!(has_batch(&storage, 1).await, "accepted batch retained");
        assert!(!has_batch(&storage, 2).await, "first unaccepted batch reverted");
        assert!(!has_batch(&storage, 3).await, "later unaccepted batch reverted");

        assert!(has_chunk(&storage, 0).await, "accepted-batch chunk retained");
        assert!(has_chunk(&storage, 1).await, "accepted-batch chunk retained");
        assert!(!has_chunk(&storage, 2).await, "unaccepted-batch chunk reverted");
        assert!(!has_chunk(&storage, 3).await, "unaccepted-batch chunk reverted");
    }

    /// Does nothing (and does not error) when no batch exists past the accepted
    /// seq_no, leaving existing artifacts untouched.
    #[tokio::test]
    async fn no_op_when_nothing_past_accepted() {
        let storage = InMemoryStorage::new_empty();
        storage.save_genesis_batch(genesis_batch()).await.unwrap();
        storage.save_next_batch(batch(1)).await.unwrap();
        storage.save_next_chunk(chunk(0, 1)).await.unwrap();

        // seq_no = 1 => first unaccepted = idx 2, which does not exist.
        let ol_client = ol_client_at_seq_no(1);
        reconcile_unaccepted_ee_artifacts(&storage, &ol_client)
            .await
            .unwrap();

        assert!(has_batch(&storage, 0).await);
        assert!(has_batch(&storage, 1).await);
        assert!(
            has_chunk(&storage, 0).await,
            "chunk untouched when nothing to revert"
        );
    }

    /// Documents the self-flagged gap in [`super::chunk_revert_boundary`]: when
    /// the first unaccepted batch has no persisted chunk linkage (still pre-DA),
    /// the chunk revert is skipped, so its chunks are left behind. A rotated
    /// chunk ELF would leave stale chunk proofs here — must be verified on the
    /// rehearsal.
    #[tokio::test]
    async fn missing_chunk_linkage_reverts_batches_but_leaves_chunks() {
        let storage = InMemoryStorage::new_empty();
        storage.save_genesis_batch(genesis_batch()).await.unwrap();
        storage.save_next_batch(batch(1)).await.unwrap();
        storage.save_next_batch(batch(2)).await.unwrap();
        for idx in 0..=2 {
            storage.save_next_chunk(chunk(idx, 2)).await.unwrap();
        }
        // Deliberately do NOT link batch 2 -> its chunks.

        let ol_client = ol_client_at_seq_no(1);
        reconcile_unaccepted_ee_artifacts(&storage, &ol_client)
            .await
            .unwrap();

        assert!(has_batch(&storage, 1).await, "accepted batch retained");
        assert!(!has_batch(&storage, 2).await, "unaccepted batch reverted");
        // Known gap: chunks survive because the linkage was missing.
        assert!(has_chunk(&storage, 0).await);
        assert!(has_chunk(&storage, 1).await);
        assert!(
            has_chunk(&storage, 2).await,
            "orphaned chunk left behind (flagged gap)"
        );
    }

    /// Never reverts the genesis batch, and reverts everything else when only
    /// genesis has been accepted (seq_no = 0).
    #[tokio::test]
    async fn genesis_is_never_reverted_when_only_genesis_accepted() {
        let storage = InMemoryStorage::new_empty();
        storage.save_genesis_batch(genesis_batch()).await.unwrap();
        storage.save_next_batch(batch(1)).await.unwrap();
        storage.save_next_batch(batch(2)).await.unwrap();
        storage.save_next_chunk(chunk(0, 1)).await.unwrap();
        storage.save_next_chunk(chunk(1, 1)).await.unwrap();
        storage
            .set_batch_chunks(batch(1).id(), vec![chunk(0, 1).id(), chunk(1, 1).id()])
            .await
            .unwrap();

        // seq_no = 0 => only genesis accepted; first unaccepted = idx 1.
        let ol_client = ol_client_at_seq_no(0);
        reconcile_unaccepted_ee_artifacts(&storage, &ol_client)
            .await
            .unwrap();

        assert!(has_batch(&storage, 0).await, "genesis must always survive");
        assert!(!has_batch(&storage, 1).await);
        assert!(!has_batch(&storage, 2).await);
        assert!(!has_chunk(&storage, 0).await);
        assert!(!has_chunk(&storage, 1).await);
    }
}
