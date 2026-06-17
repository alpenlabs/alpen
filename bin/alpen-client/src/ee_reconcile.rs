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
