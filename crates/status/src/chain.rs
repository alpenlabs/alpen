//! Container for chain status.

use std::sync::Arc;

use strata_primitives::{epoch::EpochCommitment, l2::L2BlockCommitment, prelude::*};
use strata_state::chain_state::Chainstate;

/// Describes FCM state.
#[derive(Copy, Clone, Debug)]
pub struct ChainSyncStatus {
    /// The current chain tip.
    pub tip: L2BlockCommitment,

    /// The previous epoch (ie. epoch most recently completed).
    pub prev_epoch: EpochCommitment,

    /// The finalized epoch, ie what's witnessed on L1.
    pub finalized_epoch: EpochCommitment,

    /// The last L1 block we've observed.
    pub safe_l1: L1BlockCommitment,
}

impl ChainSyncStatus {
    pub const fn tip_slot(&self) -> u64 {
        self.tip.slot()
    }

    pub const fn tip_blkid(&self) -> &L2BlockId {
        self.tip.blkid()
    }

    pub const fn finalized_blkid(&self) -> &L2BlockId {
        self.finalized_epoch.last_blkid()
    }

    pub const fn cur_epoch(&self) -> u64 {
        self.prev_epoch.epoch() + 1
    }
}

impl ChainSyncStatus {
    pub fn new(
        tip: L2BlockCommitment,
        prev_epoch: EpochCommitment,
        finalized_epoch: EpochCommitment,
        safe_l1: L1BlockCommitment,
    ) -> Self {
        Self {
            tip,
            prev_epoch,
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

    pub const fn new_status(&self) -> ChainSyncStatus {
        self.new_status
    }

    pub const fn new_tl_chainstate(&self) -> &Arc<Chainstate> {
        &self.new_tl_chainstate
    }

    /// Returns the current epoch.
    pub const fn cur_epoch(&self) -> u64 {
        self.new_status().cur_epoch()
    }
}
