use std::sync::Arc;

use async_trait::async_trait;
use strata_db_types::{ol_block::BlockStatus, DbResult};
use strata_identifiers::{Epoch, Slot};
use strata_ol_block_ancestry_provider::BlockAncestryError;
use strata_ol_chain_types_v1::OLBlockHeaderV1;
use strata_ol_state_container::OLStateContainer;
use strata_primitives::{epoch::EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_status::OLSyncStatus;

use crate::{
    ol_mmr_reconcile::{OLMmrReconcileResult, OLMmrReconcileTarget},
    unfinalized_tracker::UnfinalizedOLBlockSource,
};

/// Chain execution operations required by FCM.
#[async_trait]
pub trait ChainController: Send + Sync {
    async fn try_exec_block(&self, block: OLBlockCommitment) -> anyhow::Result<()>;
    /// Notifies the chain worker after FCM finishes processing and marks a block valid.
    async fn handle_accepted_block(&self, block: OLBlockCommitment) -> anyhow::Result<()>;
    /// Notifies the chain worker after FCM marks a block invalid.
    async fn handle_rejected_block(&self, block: OLBlockCommitment) -> anyhow::Result<()>;
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

    /// Returns the base of locally retained full OL block history, if any.
    ///
    /// Promoted checkpoint-sync datadirs can start from this anchor with the
    /// anchor header/state present while older intra-epoch parents are pruned.
    async fn get_history_base(&self) -> DbResult<Option<EpochCommitment>>;

    /// Rolls back block-attributed OL state-indexing writes in `epoch` to `cutoff`.
    async fn rollback_block_state_indexing(
        &self,
        epoch: Epoch,
        cutoff: OLBlockCommitment,
    ) -> DbResult<()>;

    async fn get_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<Arc<OLStateContainer>>>;

    async fn get_canonical_block_at(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>>;

    /// Replaces the canonical suffix from `start_slot` with `block_ids`.
    async fn replace_canonical_suffix_from(
        &self,
        start_slot: Slot,
        block_ids: Vec<OLBlockId>,
    ) -> DbResult<()>;
}

/// Startup reconciliation operations required by FCM.
#[async_trait]
pub trait FcmStartupReconciler: Send + Sync {
    /// Reconciles storage-derived indexes to FCM's selected startup tip.
    ///
    /// Called after FCM has repaired the canonical block index and loaded the
    /// selected tip's OL state, before the service launches and replays startup
    /// candidates.
    async fn reconcile_ol_mmr_index(
        &self,
        target: OLMmrReconcileTarget,
    ) -> OLMmrReconcileResult<()>;
}

/// FCM's dependency context.
#[async_trait]
pub trait FcmContext:
    ChainController + CsmStatusReader + FcmStorage + FcmStartupReconciler + Send + Sync + 'static
{
    /// Finds the previous epoch's terminal on `start`'s branch, excluding `start`.
    ///
    /// Missing or inconsistent ancestry and epoch-zero starts return an error.
    async fn find_previous_epoch_terminal_block(
        &self,
        start: &OLBlockHeaderV1,
    ) -> Result<OLBlockCommitment, BlockAncestryError>;

    fn publish_sync_status(&self, status: OLSyncStatus);
}
