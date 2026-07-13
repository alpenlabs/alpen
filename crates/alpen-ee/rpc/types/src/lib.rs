//! Alpen EE RPC type definitions.

use serde::{Deserialize, Serialize};
use strata_identifiers::Epoch;

/// L1 finalization status of an EE block.
///
/// The `status` discriminant serializes as a lowercase string.
///
/// JSON representation:
/// - `{"status": "pending"}`
/// - `{"status": "confirmed"}`
/// - `{"status": "finalized"}`
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

    /// OL checkpoint epoch that contains this block.
    ///
    /// Pending blocks are not covered by any checkpoint and carry no epoch. Confirmed and
    /// finalized blocks carry the per-block containing checkpoint epoch, not the node's frontier
    /// epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_epoch: Option<Epoch>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_status_response_json_shape() {
        let pending = BlockStatusResponse {
            status: BlockStatus::Pending,
            checkpoint_epoch: None,
        };
        assert_eq!(
            serde_json::to_value(pending).unwrap(),
            serde_json::json!({ "status": "pending" }),
        );

        let confirmed = BlockStatusResponse {
            status: BlockStatus::Confirmed,
            checkpoint_epoch: Some(5),
        };
        assert_eq!(
            serde_json::to_value(confirmed).unwrap(),
            serde_json::json!({ "status": "confirmed", "checkpoint_epoch": 5 }),
        );

        let finalized = BlockStatusResponse {
            status: BlockStatus::Finalized,
            checkpoint_epoch: Some(0),
        };
        assert_eq!(
            serde_json::to_value(finalized).unwrap(),
            serde_json::json!({ "status": "finalized", "checkpoint_epoch": 0 }),
        );
    }

    #[test]
    fn block_status_response_round_trips() {
        for response in [
            BlockStatusResponse {
                status: BlockStatus::Pending,
                checkpoint_epoch: None,
            },
            BlockStatusResponse {
                status: BlockStatus::Confirmed,
                checkpoint_epoch: Some(7),
            },
            BlockStatusResponse {
                status: BlockStatus::Finalized,
                checkpoint_epoch: Some(42),
            },
        ] {
            let encoded = serde_json::to_string(&response).unwrap();
            let decoded: BlockStatusResponse = serde_json::from_str(&encoded).unwrap();
            assert_eq!(response, decoded);
        }
    }
}
