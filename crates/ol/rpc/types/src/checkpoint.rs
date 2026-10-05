use serde::{Deserialize, Serialize};
use strata_db_types::ol_checkpoint::RejectedCheckpointEntry;
use strata_identifiers::{Epoch, EpochCommitment, L1BlockCommitment, L2BlockCommitment, RBuf32};

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
///
/// `txid` and `wtxid` use [`RBuf32`] so JSON serialization matches Bitcoin's
/// reversed-byte convention used by block explorers and `bitcoin-cli`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcCheckpointL1Ref {
    /// L1 block commitment where the checkpoint was observed.
    pub l1_block: L1BlockCommitment,
    /// Txid of checkpoint transaction.
    pub txid: RBuf32,
    /// Wtxid of checkpoint transaction.
    pub wtxid: RBuf32,
}

impl RpcCheckpointL1Ref {
    /// Creates a new [`RpcCheckpointL1Ref`].
    pub fn new(l1_block: L1BlockCommitment, txid: RBuf32, wtxid: RBuf32) -> Self {
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
    /// First L2 block of the checkpoint. `None` on checkpoint-sync nodes for
    /// non-genesis epochs, where deriving it needs block bodies the node lacks.
    pub l2_start: Option<L2BlockCommitment>,
    /// Last L2 block (terminal) of the checkpoint.
    pub l2_end: L2BlockCommitment,
    /// Confirmation/finality status.
    pub confirmation_status: RpcCheckpointConfStatus,
}

/// A checkpoint transaction this node submitted that was mined on L1 but not accepted by the ASM.
///
/// Reported once its block is buried at the node's L1 reorg-safe depth while the ASM's verified
/// epoch is still below the checkpoint's epoch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcRejectedCheckpoint {
    /// Epoch the checkpoint declared.
    pub epoch: Epoch,
    /// Epoch commitment the checkpoint declared.
    pub commitment: EpochCommitment,
    /// Txid of the checkpoint transaction.
    pub txid: RBuf32,
    /// L1 block the transaction was mined in.
    pub l1_block: L1BlockCommitment,
    /// ASM verified tip when the rejection was recorded, `None` before any accepted checkpoint.
    pub asm_verified_tip: Option<EpochCommitment>,
}

impl From<RejectedCheckpointEntry> for RpcRejectedCheckpoint {
    fn from(entry: RejectedCheckpointEntry) -> Self {
        Self {
            epoch: entry.epoch(),
            commitment: entry.commitment(),
            txid: entry.txid(),
            l1_block: entry.l1_block(),
            asm_verified_tip: entry.asm_verified_tip(),
        }
    }
}
