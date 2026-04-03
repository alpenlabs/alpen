use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, L1BlockCommitment, L2BlockCommitment};

/// RPC checkpoint confirmation status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RpcCheckpointConfStatus {
    /// Checkpoint has not been observed on L1 yet.
    Pending,
    /// Checkpoint is observed on L1 but not finalized by depth.
    Confirmed {
        /// L1 transaction reference where checkpoint was observed.
        l1_reference: RpcCheckpointL1Ref,
    },
    /// Checkpoint is finalized by L1 depth.
    Finalized {
        /// L1 transaction reference where checkpoint was observed.
        l1_reference: RpcCheckpointL1Ref,
    },
}

/// Reference to the L1 transaction carrying the checkpoint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcCheckpointL1Ref {
    /// L1 block commitment where the checkpoint was observed.
    pub l1_block: L1BlockCommitment,
    /// Txid of checkpoint transaction.
    pub txid: Buf32,
    /// Wtxid of checkpoint transaction.
    pub wtxid: Buf32,
}

impl RpcCheckpointL1Ref {
    /// Creates a new [`RpcCheckpointL1Ref`].
    pub fn new(l1_block: L1BlockCommitment, txid: Buf32, wtxid: Buf32) -> Self {
        Self {
            l1_block,
            txid,
            wtxid,
        }
    }
}

/// OL-native checkpoint info response.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcCheckpointInfo {
    /// Checkpoint index (epoch).
    pub idx: u64,
    /// L1 block range (inclusive) covered by the checkpoint.
    pub l1_range: (L1BlockCommitment, L1BlockCommitment),
    /// L2 block range (inclusive) covered by the checkpoint.
    pub l2_range: (L2BlockCommitment, L2BlockCommitment),
    /// Confirmation/finality status.
    pub confirmation_status: RpcCheckpointConfStatus,
}
