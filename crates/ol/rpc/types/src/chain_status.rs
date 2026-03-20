use serde::{Deserialize, Serialize};
use strata_identifiers::{Epoch, EpochCommitment, OLBlockId, Slot};

/// RPC representation of the current OL tip block header.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLBlockInfo {
    /// Block id.
    pub blkid: OLBlockId,

    /// Slot.
    pub slot: Slot,

    /// Epoch.
    pub epoch: Epoch,

    /// Whether the block is epoch terminal.
    pub is_terminal: bool,
}

impl RpcOLBlockInfo {
    /// Creates a new [`RpcOLBlockInfo`].
    pub fn new(blkid: OLBlockId, slot: Slot, epoch: Epoch, is_terminal: bool) -> Self {
        Self {
            blkid,
            slot,
            epoch,
            is_terminal,
        }
    }

    /// Returns the block id.
    pub fn blkid(&self) -> OLBlockId {
        self.blkid
    }

    /// Returns the slot.
    pub fn slot(&self) -> Slot {
        self.slot
    }

    /// Returns the epoch.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Returns whether the block is epoch terminal.
    pub fn is_terminal(&self) -> bool {
        self.is_terminal
    }
}

/// OL chain status with tip block info, confirmed epoch and finalized epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct RpcOLChainStatus {
    /// Tip block info.
    pub tip: RpcOLBlockInfo,

    /// Confirmed epoch commitment.
    pub confirmed: EpochCommitment,

    /// Finalized epoch commitment.
    pub finalized: EpochCommitment,
}

impl RpcOLChainStatus {
    /// Creates a new [`RpcOLChainStatus`].
    pub fn new(
        tip: RpcOLBlockInfo,
        confirmed: EpochCommitment,
        finalized: EpochCommitment,
    ) -> Self {
        Self {
            tip,
            confirmed,
            finalized,
        }
    }

    /// Returns the tip block info.
    pub fn tip(&self) -> &RpcOLBlockInfo {
        &self.tip
    }

    /// Returns the confirmed epoch commitment.
    pub fn confirmed(&self) -> &EpochCommitment {
        &self.confirmed
    }

    /// Returns the finalized epoch commitment.
    pub fn finalized(&self) -> &EpochCommitment {
        &self.finalized
    }
}
