//! Alpen EE RPC type definitions.

use serde::{Deserialize, Serialize};

/// L1 finalization status of an EE block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct BlockStatusResponse {
    /// L1 finalization status.
    pub status: BlockStatus,
}

/// Response for `alpen_getChunkProofCoverage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct ChunkProofCoverageResponse {
    /// First requested EE block number.
    pub start_block: u64,

    /// Last requested EE block number.
    pub end_block: u64,

    /// True when proof-ready chunks cover every block in the requested range.
    pub covered: bool,

    /// First requested block not yet covered by a proof-ready chunk.
    pub first_uncovered_block: Option<u64>,
}
