//! Alpen EE RPC type definitions.

use serde::{Deserialize, Serialize};

/// L1 finalization status of an EE block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockStatus {
    /// Block is not yet covered by any confirmed or finalized checkpoint.
    Pending,

    /// Block is covered by a confirmed OL checkpoint.
    Confirmed,

    /// Block is covered by a finalized OL checkpoint.
    Finalized,
}

/// Response for `alpen_getBlockStatus`.
///
/// Reserved for forward-compatible expansion; additional fields may be added without changing the
/// method signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockStatusResponse {
    /// L1 finalization status.
    pub status: BlockStatus,
}

/// Storage-backed status of an EE batch in the proving pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProofPipelineBatchStatus {
    /// Genesis batch.
    Genesis,
    /// Batch is sealed and ready for DA posting.
    Sealed,
    /// DA has been requested.
    DaPending,
    /// DA has completed.
    DaComplete,
    /// Batch proof generation has been requested.
    ProofPending,
    /// Batch proof is ready.
    ProofReady,
}

/// Storage-backed status of an EE chunk in the proving pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProofPipelineChunkStatus {
    /// Chunk proving has not started.
    ProvingNotStarted,
    /// Chunk proof generation has been requested.
    ProofPending,
    /// Chunk proof is ready.
    ProofReady,
}

/// Batch summary returned by `alpen_getProofPipelineStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofPipelineBatch {
    /// Sequential batch index.
    pub idx: u64,
    /// Last block hash in the batch.
    pub last_block: String,
    /// Last block number in the batch.
    pub last_block_number: u64,
    /// Current batch status.
    pub status: ProofPipelineBatchStatus,
    /// Proof id when status is proof-ready.
    pub proof: Option<String>,
}

/// Chunk summary returned by `alpen_getProofPipelineStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofPipelineChunk {
    /// Sequential chunk index.
    pub idx: u64,
    /// Last block hash in the chunk.
    pub last_block: String,
    /// Last block number in the chunk when it is still canonical locally.
    pub last_block_number: Option<u64>,
    /// Current chunk status.
    pub status: ProofPipelineChunkStatus,
    /// Proof id when status is proof-ready.
    pub proof: Option<String>,
}

/// Response for `alpen_getProofPipelineStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofPipelineStatusResponse {
    /// Latest batch known to EE storage.
    pub latest_batch: Option<ProofPipelineBatch>,
    /// Latest proof-ready batch known to EE storage.
    pub latest_proof_ready_batch: Option<ProofPipelineBatch>,
    /// Latest chunk known to EE storage.
    pub latest_chunk: Option<ProofPipelineChunk>,
    /// Latest proof-ready chunk known to EE storage.
    pub latest_proof_ready_chunk: Option<ProofPipelineChunk>,
}
