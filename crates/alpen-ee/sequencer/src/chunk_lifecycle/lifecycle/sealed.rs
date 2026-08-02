use alpen_ee_common::{BatchStorage, Chunk, ChunkProver, ChunkStorage};
use eyre::Result;
use tracing::{debug, instrument, warn};

use crate::chunk_lifecycle::ctx::ChunkLifecycleCtx;

#[instrument(skip_all, fields(
    chunk_id = ?chunk.id(),
    chunk_idx = chunk.idx(),
    batch_idx = chunk.batch_idx(),
))]
pub(crate) async fn try_advance_sealed<P, S>(
    ctx: &ChunkLifecycleCtx<P, S>,
    chunk: &Chunk,
) -> Result<()>
where
    P: ChunkProver,
    S: ChunkStorage + BatchStorage,
{
    let chunk_id = chunk.id();

    // The chunk builder stamps every chunk with the index of the batch still being accumulated, and
    // that batch's row only materializes in storage when the batch seals. A missing batch row is
    // therefore ambiguous: the batch may still be accumulating, or it may have been reverted by a
    // reorg whose chunk cleanup has not run yet. Both cases look identical from here, so the chunk
    // is deferred until the row exists. Deferring is the safe direction: a still-accumulating batch
    // seals soon and the chunk is picked up on a later tick, whereas proving a reverted chunk leaks
    // a prover task record and a receipt that nothing ever reclaims, and can raise a delayed
    // permanent-failure alert for a chunk that no longer exists.
    let Some((batch, _status)) = ctx.storage.get_batch_by_idx(chunk.batch_idx()).await? else {
        debug!("deferring sealed chunk whose batch is not sealed yet or was reverted");
        return Ok(());
    };

    // The owning batch has sealed, so its chunk linkage is authoritative once written.
    if let Some(batch_chunks) = ctx.storage.get_batch_chunks(batch.id()).await? {
        if !batch_chunks.contains(&chunk_id) {
            let batch_id = batch.id();
            debug!(%batch_id, "skipping sealed chunk not linked to its batch");
            return Ok(());
        }
    }

    debug!("requesting chunk proof");

    match ctx.prover.request_proof_generation(chunk_id).await {
        Ok(()) => Ok(()),
        Err(e) => {
            warn!(
                error = %e,
                "failed to request chunk proof; retrying on next lifecycle tick"
            );
            Ok(())
        }
    }
}
