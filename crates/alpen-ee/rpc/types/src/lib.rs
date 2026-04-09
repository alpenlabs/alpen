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

/// Response for `strataee_getBlockStatus`.
///
/// Reserved for forward-compatible expansion; additional fields may be added without changing the
/// method signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockStatusResponse {
    /// L1 finalization status.
    pub status: BlockStatus,
}
