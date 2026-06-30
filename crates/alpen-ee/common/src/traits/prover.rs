use async_trait::async_trait;

use crate::{BatchId, ChunkId, Proof, ProofId};

#[derive(Debug, Clone)]
pub enum ProofGenerationStatus {
    /// Proof generation requested and proof is getting generated.
    /// Temporary failure are retried internally while status remains pending.
    Pending,
    /// Proof is ready and can be fetched using proof_id.
    Ready { proof_id: ProofId },
    /// Proof generation has not been requested for provided batch_id.
    NotStarted,
    /// Permanent failure that indicates the given batch can never be proven.
    /// Needs manual intervention to resolve.
    Failed { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofRequestStatus {
    /// A new proof task was submitted.
    Submitted,
    /// A proof task or persisted proof already exists.
    ///
    /// The task may be pending, running, completed, or permanently failed; callers should use
    /// [`BatchProver::check_proof_status`] when they need the concrete task outcome.
    AlreadyExists,
    /// The prover did not submit because required inputs are not available yet.
    WaitingForInputs,
}

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait ChunkProver {
    /// Request proof generation for a sealed chunk.
    async fn request_proof_generation(&self, chunk_id: ChunkId) -> eyre::Result<()>;

    /// Check the prover task status for a chunk proof.
    async fn check_proof_status(&self, chunk_id: ChunkId) -> eyre::Result<ProofGenerationStatus>;
}

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait BatchProver {
    /// Request acct proof generation for batch_id.
    async fn request_proof_generation(&self, batch_id: BatchId)
        -> eyre::Result<ProofRequestStatus>;

    /// Check if acct proof is generated for batch_id.
    ///
    /// The generated proof is expected to be persisted, available to be fetched at any time
    /// afterwards with the returned proof_id.
    async fn check_proof_status(&self, batch_id: BatchId) -> eyre::Result<ProofGenerationStatus>;

    /// Get a previously generated proof by id.
    ///
    /// None -> proofId not found
    async fn get_proof(&self, proof_id: ProofId) -> eyre::Result<Option<Proof>>;
}
