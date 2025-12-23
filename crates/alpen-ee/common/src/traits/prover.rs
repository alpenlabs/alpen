use async_trait::async_trait;

use crate::{EeUpdateId, Proof, ProofId};

#[derive(Debug)]
pub enum ProofGenerationStatus {
    /// Proof generation requested and proof is getting generated.
    Pending,
    /// Proof is ready and can be fetched using proof_id.
    Ready { proof_id: ProofId },
    /// Proof generation has not been requested for provided ee_update_id.
    NotStarted,
    /// Cannot generate proof for some reason. All retries exhausted, etc.
    Failed { reason: String },
}

/// Interface between Prover and Batch assembly
#[async_trait]
pub trait EeUpdateProver: Sized {
    /// Request proof generation for ee_update_id.
    /// Ok(()) -> proof generation has been queued
    async fn request_proof_generation(&self, ee_update_id: EeUpdateId) -> eyre::Result<()>;

    /// Check if proof is generated for ee_update_id.
    ///
    /// The generated proof is expected to be persisted, available to be fetched at any time
    /// afterwards with the returned proof_id.
    async fn check_proof_status(
        &self,
        ee_update_id: EeUpdateId,
    ) -> eyre::Result<ProofGenerationStatus>;

    /// Get a previously generated proof by id.
    ///
    /// None -> proofId not found
    async fn get_proof(&self, proof_id: ProofId) -> eyre::Result<Option<Proof>>;
}
