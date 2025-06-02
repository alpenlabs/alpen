use strata_consensus_logic::unfinalized_tracker::UnfinalizedBlockTracker;
use strata_primitives::{epoch::EpochCommitment, l2::L2BlockCommitment};
use strata_state::{
    header::{L2Header, SignedL2BlockHeader},
    id::L2BlockId,
};
use strata_storage::NodeStorage;
use tracing::debug;

use crate::L2SyncError;

#[derive(Debug)]
pub(crate) struct L2SyncState {
    /// Height of highest unfinalized block in tracker
    tip_block: L2BlockCommitment,

    // TODO make this just subscribe to FCM tip updates and go from there?
    tracker: UnfinalizedBlockTracker,
}

impl L2SyncState {
    pub(crate) fn attach_block(
        &mut self,
        block_header: &SignedL2BlockHeader,
    ) -> Result<(), L2SyncError> {
        self.tracker
            .attach_block(block_header.get_blockid(), block_header)?;

        // FIXME this isn't quite right, we should be following the fork choice manager
        self.tip_block = self
            .tracker
            .chain_tip_blocks_iter()
            .max_by_key(|bc| bc.slot())
            .expect("sync: picking new tip");

        Ok(())
    }

    pub(crate) fn update_finalized_tip(
        &mut self,
        epoch: EpochCommitment,
    ) -> Result<(), L2SyncError> {
        self.tracker.update_finalized_epoch(&epoch)?;
        Ok(())
    }

    pub(crate) fn has_block(&self, block_id: &L2BlockId) -> bool {
        self.tracker.is_seen_block(block_id)
    }

    // TODO rename to slot
    pub(crate) fn finalized_height(&self) -> u64 {
        self.tracker.finalized_epoch().last_slot()
    }

    pub(crate) fn finalized_blockid(&self) -> &L2BlockId {
        self.tracker.finalized_epoch().last_blkid()
    }

    // TODO rename to slot
    pub(crate) fn tip_height(&self) -> u64 {
        self.tip_block.slot()
    }

    pub(crate) fn finalized_epoch(&self) -> &EpochCommitment {
        self.tracker.finalized_epoch()
    }
}

pub(crate) async fn initialize_from_db(
    finalized_epoch: EpochCommitment,
    storage: &NodeStorage,
) -> Result<L2SyncState, L2SyncError> {
    debug!(?finalized_epoch, "loading unfinalized blocks");

    let l2man_tracker = storage.l2().clone();

    let tracker = tokio::runtime::Handle::current()
        .spawn_blocking(move || {
            let mut tracker = UnfinalizedBlockTracker::new_empty(finalized_epoch);
            tracker
                .load_unfinalized_blocks(&l2man_tracker)
                .map(|_| tracker)
        })
        .await
        .map_err(|err| L2SyncError::LoadUnfinalizedFailed(err.to_string()))?
        .map_err(|err| L2SyncError::LoadUnfinalizedFailed(err.to_string()))?;

    let tip_block = tracker
        .chain_tip_blocks_iter()
        .max_by_key(|bc| bc.slot())
        .expect("sync: missing init chain tip");

    debug!(?tip_block, "sync state initialized");

    let state = L2SyncState { tip_block, tracker };

    Ok(state)
}
