use std::sync::{Arc, RwLock};

use strata_consensus_logic::fork_choice_manager::ForkChoiceManager;
use strata_ol_chain_types::L2BlockId;
use strata_primitives::epoch::EpochCommitment;

pub(crate) struct L2SyncState {
    /// Reference to the fork choice manager to access the chain tracker
    fcm: Arc<RwLock<ForkChoiceManager>>,
}

impl L2SyncState {
    pub(crate) fn has_block(&self, block_id: &L2BlockId) -> bool {
        let fcm_guard = self.fcm.read().expect("fcm read lock poisoned");
        fcm_guard.is_chain_tracker_block_seen(block_id)
    }

    // TODO rename to slot
    pub(crate) fn finalized_height(&self) -> u64 {
        let fcm_guard = self.fcm.read().expect("fcm read lock poisoned");
        fcm_guard.get_chain_tracker_finalized_epoch().last_slot()
    }

    pub(crate) fn finalized_blockid(&self) -> L2BlockId {
        let fcm_guard = self.fcm.read().expect("fcm read lock poisoned");
        *fcm_guard.get_chain_tracker_finalized_epoch().last_blkid()
    }

    // TODO rename to slot
    pub(crate) fn tip_height(&self) -> u64 {
        let fcm_guard = self.fcm.read().expect("fcm read lock poisoned");
        fcm_guard.cur_best_block().slot()
    }

    pub(crate) fn finalized_epoch(&self) -> EpochCommitment {
        let fcm_guard = self.fcm.read().expect("fcm read lock poisoned");
        *fcm_guard.get_chain_tracker_finalized_epoch()
    }
}

pub(crate) fn new(fcm: Arc<RwLock<ForkChoiceManager>>) -> L2SyncState {
    L2SyncState { fcm }
}
