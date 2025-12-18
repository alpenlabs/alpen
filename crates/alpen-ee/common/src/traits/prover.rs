use async_trait::async_trait;

use crate::{EeUpdateId, Proof, ProofId};

#[derive(Debug)]
pub enum ProofGenerationStatus {
    /// Proof generation started or proof is getting generated
    Pending,
    /// Proof is ready and can be fetched using proof_id
    Ready { proof_id: ProofId },
    /// Proof generation has not been triggered for provided ee_update_id.
    Unproven,
    /// Cannot generate proof for some reason. All retries exhausted, etc.
    Failed { reason: String },
}

/// Interface between Prover and Batch assembly
#[async_trait]
pub trait EeUpdateProver: Sized {
    /// Request proof generation for ee_update_id.
    /// Ok(()) -> proof generation has been queued
    async fn begin_proof_generation(&self, ee_update_id: EeUpdateId) -> eyre::Result<()>;

    /// Check if proof is generated for ee_update_id.
    ///
    /// The generated proof is expected to be persisted, available to be fetched at any time
    /// afterwards with the returned proof_id.
    async fn check_proof_ready(
        &self,
        ee_update_id: EeUpdateId,
    ) -> eyre::Result<ProofGenerationStatus>;

    /// Get a proof from persistence storage.
    ///
    /// None -> proofId not found
    async fn get_proof(&self, proof_id: ProofId) -> eyre::Result<Option<Proof>>;
}
