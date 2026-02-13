//! Container for chain status.

use std::sync::Arc;

use strata_identifiers::Epoch;
use strata_ol_chainstate_types::Chainstate;
use strata_primitives::{epoch::EpochCommitment, l2::L2BlockCommitment, prelude::*};

/// Type alias for ChainSyncStatus that indicates this is OL specific even though the field types
/// are same.
pub type OLSyncStatus = ChainSyncStatus;

/// Describes FCM state.
#[derive(Copy, Clone, Debug)]
pub struct ChainSyncStatus {
    /// The current chain tip.
    pub tip: L2BlockCommitment,

    /// The previous epoch (ie. epoch most recently completed).
    pub prev_epoch: EpochCommitment,

    /// The last epoch confirmed by the ASM on L1 (ie. checkpoint posted and accepted).
    pub confirmed_epoch: EpochCommitment,

    /// The finalized epoch, ie what's witnessed on L1.
    pub finalized_epoch: EpochCommitment,

    /// The last L1 block we've observed.
    pub safe_l1: L1BlockCommitment,
}

impl ChainSyncStatus {
    pub fn tip_slot(&self) -> u64 {
        self.tip.slot()
    }

    pub fn tip_blkid(&self) -> &L2BlockId {
        self.tip.blkid()
    }

    pub fn finalized_blkid(&self) -> &L2BlockId {
        self.finalized_epoch.last_blkid()
    }

    pub fn cur_epoch(&self) -> Epoch {
        self.prev_epoch.epoch() + 1
    }
}

impl ChainSyncStatus {
    pub fn new(
        tip: L2BlockCommitment,
        prev_epoch: EpochCommitment,
        confirmed_epoch: EpochCommitment,
        finalized_epoch: EpochCommitment,
        safe_l1: L1BlockCommitment,
    ) -> Self {
        Self {
            tip,
            prev_epoch,
            confirmed_epoch,
            finalized_epoch,
            safe_l1,
        }
    }
}

/// Published to the FCM status including chainstate.
#[derive(Debug, Clone)]
pub struct ChainSyncStatusUpdate {
    new_status: ChainSyncStatus,
    new_tl_chainstate: Arc<Chainstate>,
}

impl ChainSyncStatusUpdate {
    pub fn new(new_status: ChainSyncStatus, new_tl_chainstate: Arc<Chainstate>) -> Self {
        Self {
            new_status,
            new_tl_chainstate,
        }
    }

    pub fn new_status(&self) -> ChainSyncStatus {
        self.new_status
    }

    pub fn new_tl_chainstate(&self) -> &Arc<Chainstate> {
        &self.new_tl_chainstate
    }

    /// Returns the current epoch.
    pub fn cur_epoch(&self) -> Epoch {
        self.new_status().cur_epoch()
    }
}

/// OL update published via FCM.
#[derive(Debug, Clone)]
pub struct OLSyncStatusUpdate {
    new_status: OLSyncStatus,
}

impl OLSyncStatusUpdate {
    pub fn new(new_status: OLSyncStatus) -> Self {
        Self { new_status }
    }

    pub fn new_status(&self) -> OLSyncStatus {
        self.new_status
    }
}
