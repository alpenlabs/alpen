use std::sync::Arc;

use async_trait::async_trait;
use strata_db_types::{traits::BlockStatus, DbResult};
use strata_identifiers::{Epoch, Slot};
use strata_ol_state_types::OLState;
use strata_primitives::{epoch::EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_status::OLSyncStatus;

use crate::unfinalized_tracker::UnfinalizedOLBlockSource;

/// Chain execution operations required by FCM.
#[async_trait]
pub trait ChainController: Send + Sync {
    async fn try_exec_block(&self, block: OLBlockCommitment) -> anyhow::Result<()>;
    async fn update_safe_tip(&self, safe_tip: OLBlockCommitment) -> anyhow::Result<()>;
    async fn finalize_epoch(&self, epoch: EpochCommitment) -> anyhow::Result<()>;
}

/// CSM status access required by FCM.
pub trait CsmStatusReader: Send + Sync {
    fn last_finalized_epoch(&self) -> Option<EpochCommitment>;
    fn last_confirmed_epoch(&self) -> Option<EpochCommitment>;
}

/// Storage operations required by FCM.
#[async_trait]
pub trait FcmStorage: UnfinalizedOLBlockSource {
    async fn set_block_status(&self, blkid: OLBlockId, status: BlockStatus) -> DbResult<bool>;

    async fn clear_block_high_watermark(&self, expected: OLBlockCommitment) -> DbResult<bool>;

    /// Returns the latest OL block committed through the high-watermark path,
    /// if any.
    async fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>>;

    /// Rolls back per-block OL state-indexing writes in `epoch` attributed to
    /// blocks with slots strictly greater than `cutoff.slot()`.
    ///
    /// Called when a block is marked invalid to drop its indexing writes, so
    /// a replacement block at the same slot can apply its own without
    /// conflicting against the indexing high-watermark. Idempotent; a no-op
    /// when the rejected block never reached the persist step.
    async fn rollback_block_state_indexing(
        &self,
        epoch: Epoch,
        cutoff: OLBlockCommitment,
    ) -> DbResult<()>;

    /// Deletes the epoch summary keyed by exactly this epoch commitment.
    ///
    /// Called when a terminal block is marked invalid to drop the summary it
    /// may have stored before failing, so a stale summary cannot shadow the
    /// replacement terminal's summary in canonical epoch lookups. Returns
    /// `true` when a summary existed and was deleted.
    async fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool>;

    async fn get_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<Arc<OLState>>>;

    async fn get_canonical_block_at(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>>;

    /// Replaces the canonical index suffix above `pivot_slot` with `blocks`.
    ///
    /// Single write path for canonical-tip changes: truncates every canonical entry above
    /// `pivot_slot`, then writes `blocks`, atomically. An extend passes an empty truncation; a
    /// reorg truncates the abandoned branch and writes the new one; a revert passes an empty
    /// `blocks`.
    async fn replace_canonical_suffix(
        &self,
        pivot_slot: Slot,
        blocks: Vec<(Slot, OLBlockId)>,
    ) -> DbResult<()>;

    async fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>>;
}

/// FCM's dependency context.
pub trait FcmContext:
    ChainController + CsmStatusReader + FcmStorage + Send + Sync + 'static
{
    fn publish_sync_status(&self, status: OLSyncStatus);
}
