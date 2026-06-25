use alpen_ee_common::{BatchStorage, Chunk, ChunkProver, ChunkStorage};
use eyre::Result;
use tracing::{debug, warn};

use crate::chunk_lifecycle::ctx::ChunkLifecycleCtx;

pub(crate) async fn try_advance_sealed<P, S>(
    ctx: &ChunkLifecycleCtx<P, S>,
    chunk: &Chunk,
) -> Result<()>
where
    P: ChunkProver,
    S: ChunkStorage + BatchStorage,
{
    let chunk_id = chunk.id();
    let Some((batch, _status)) = ctx.storage.get_batch_by_idx(chunk.batch_idx()).await? else {
        debug!(
            ?chunk_id,
            chunk_idx = chunk.idx(),
            batch_idx = chunk.batch_idx(),
            "skipping sealed chunk whose batch was reverted"
        );
        return Ok(());
    };

    if let Some(batch_chunks) = ctx.storage.get_batch_chunks(batch.id()).await? {
        if !batch_chunks.contains(&chunk_id) {
            let batch_id = batch.id();
            debug!(
                ?chunk_id,
                %batch_id,
                chunk_idx = chunk.idx(),
                batch_idx = chunk.batch_idx(),
                "skipping sealed chunk not linked to its batch"
            );
            return Ok(());
        }
    }

    debug!(
        ?chunk_id,
        chunk_idx = chunk.idx(),
        batch_idx = chunk.batch_idx(),
        "requesting chunk proof"
    );

    match ctx.prover.request_proof_generation(chunk_id).await {
        Ok(()) => Ok(()),
        Err(e) => {
            warn!(
                ?chunk_id,
                chunk_idx = chunk.idx(),
                error = %e,
                "failed to request chunk proof; retrying on next lifecycle tick"
            );
            Ok(())
        }
    }
}
