use alpen_ee_common::{
    BatchStorage, Chunk, ChunkProver, ChunkStatus, ChunkStorage, ProofGenerationStatus,
};
use eyre::Result;
use tracing::{debug, error, warn};

use super::sealed::try_advance_sealed;
use crate::chunk_lifecycle::ctx::ChunkLifecycleCtx;

pub(crate) async fn try_advance_proof_pending<P, S>(
    ctx: &ChunkLifecycleCtx<P, S>,
    chunk: &Chunk,
) -> Result<()>
where
    P: ChunkProver,
    S: ChunkStorage + BatchStorage,
{
    let chunk_id = chunk.id();
    match ctx.prover.check_proof_status(chunk_id).await? {
        ProofGenerationStatus::Ready { proof_id } => {
            debug!(
                ?chunk_id,
                chunk_idx = chunk.idx(),
                %proof_id,
                "chunk proof ready; updating chunk status"
            );
            ctx.storage
                .update_chunk_status(chunk_id, ChunkStatus::ProofReady(proof_id))
                .await?;
        }
        ProofGenerationStatus::Failed { reason } => {
            error!(
                ?chunk_id,
                chunk_idx = chunk.idx(),
                %reason,
                "CRITICAL: chunk proof generation failed permanently; manual intervention required"
            );
            ctx.storage
                .update_chunk_status(chunk_id, ChunkStatus::ProofFailed(reason))
                .await?;
        }
        ProofGenerationStatus::NotStarted => {
            warn!(
                ?chunk_id,
                chunk_idx = chunk.idx(),
                "chunk status is ProofPending but PaaS task is missing; re-submitting chunk proof"
            );
            return try_advance_sealed(ctx, chunk).await;
        }
        ProofGenerationStatus::Pending => {}
    }

    Ok(())
}
