//! Container for chain status.

use std::{fmt::Debug, sync::Arc};

use strata_identifiers::Epoch;
use strata_ol_chainstate_types::Chainstate;
use strata_ol_state_types::OLState;
use strata_primitives::{epoch::EpochCommitment, l2::L2BlockCommitment, prelude::*};

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
///
/// Generic over the state type `S` to support different state representations.
#[derive(Debug, Clone)]
pub struct ChainSyncStatusUpdate<S: Clone + Debug + Send + Sync> {
    new_status: ChainSyncStatus,
    new_tl_chainstate: Arc<S>,
}

impl<S: Clone + Debug + Send + Sync> ChainSyncStatusUpdate<S> {
    pub fn new(new_status: ChainSyncStatus, new_tl_chainstate: Arc<S>) -> Self {
        Self {
            new_status,
            new_tl_chainstate,
        }
    }

    pub fn new_status(&self) -> ChainSyncStatus {
        self.new_status
    }

    pub fn new_tl_chainstate(&self) -> &Arc<S> {
        &self.new_tl_chainstate
    }

    /// Returns the current epoch.
    pub fn cur_epoch(&self) -> Epoch {
        self.new_status().cur_epoch()
    }

    /// Returns the current tip commitment.
    pub fn tip(&self) -> L2BlockCommitment {
        self.new_status.tip
    }
}

/// L2 chain sync update - carries the full Chainstate.
///
/// Used by L2/legacy consumers that need the complete chainstate.
pub type L2ChainSyncUpdate = ChainSyncStatusUpdate<Chainstate>;

/// OL chain sync update - carries the OLState.
///
/// Used by OL-specific consumers like the mempool and block assembly.
pub type OLChainSyncUpdate = ChainSyncStatusUpdate<OLState>;
