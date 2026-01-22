use alpen_ee_common::{BatchId, BatchProver, Proof, ProofGenerationStatus, ProofId};
use async_trait::async_trait;

/// Simple implementation of [`BatchProver`] that accepts everything as Ok to allow batch lifecycle
/// to proceed with empty proofs. To be replaced once EE Prover implementation is completed.
pub(crate) struct NoopProver;

#[async_trait]
impl BatchProver for NoopProver {
    async fn request_proof_generation(&self, _batch_id: BatchId) -> eyre::Result<()> {
        Ok(())
    }

    async fn check_proof_status(&self, _batch_id: BatchId) -> eyre::Result<ProofGenerationStatus> {
        Ok(ProofGenerationStatus::Ready {
            proof_id: ProofId::zero(),
        })
    }

    async fn get_proof(&self, _proof_id: ProofId) -> eyre::Result<Option<Proof>> {
        Ok(Some(Proof::from_vec(vec![])))
    }
}
