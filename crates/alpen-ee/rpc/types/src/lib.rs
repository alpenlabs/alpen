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

/// Proof lifecycle status for a chunk range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkProofStatus {
    /// Chunk proof generation has not started.
    NotStarted,

    /// Chunk proof generation has been submitted but has not completed.
    Pending,

    /// Native or remote proving completed and the proof receipt was stored.
    ProofReady,
}

/// A chunk range and its proof status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkProofRange {
    /// Sequential chunk index.
    pub chunk_index: u64,

    /// First EE block number covered by this chunk.
    pub start_block: u64,

    /// Last EE block number covered by this chunk.
    pub end_block: u64,

    /// Chunk proof status.
    pub status: ChunkProofStatus,

    /// Chunk proof id, when the chunk proof is ready.
    pub proof_id: Option<String>,

    /// Previous block hash for the chunk.
    pub prev_block: String,

    /// Last block hash for the chunk.
    pub last_block: String,
}

/// Response for `alpen_getChunkProofCoverage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkProofCoverageResponse {
    /// First requested EE block number.
    pub start_block: u64,

    /// Last requested EE block number.
    pub end_block: u64,

    /// True when proof-ready chunks cover every block in the requested range.
    pub covered: bool,

    /// First requested block not yet covered by a proof-ready chunk.
    pub first_uncovered_block: Option<u64>,

    /// Chunk ranges that intersect the requested block range.
    pub ranges: Vec<ChunkProofRange>,
}
