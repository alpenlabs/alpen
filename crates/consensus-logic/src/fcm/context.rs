use std::{num::NonZeroUsize, sync::Arc};

use async_trait::async_trait;
use strata_db_types::{
    ol_block::{BlockStatus, StatusScanStart},
    DbResult,
};
use strata_identifiers::{Epoch, Slot};
use strata_ol_state_types_v1::OLStateV1;
use strata_primitives::{epoch::EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_status::OLSyncStatus;

use crate::{
    ol_mmr_reconcile::{OLMmrReconcileResult, OLMmrReconcileTarget},
    unfinalized_tracker::UnfinalizedOLBlockSource,
};

/// A local dependency that prevents a block execution verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionDeferral {
    /// Parent execution or canonical L1/ASM data is not ready.
    Dependency,

    /// A local storage operation failed and must be retried with backoff.
    Storage,

    /// Another block occupies the epoch's indexing position.
    ///
    /// Execution waits for indexing reconciliation, with backoff that ordinary chain
    /// progress cannot bypass. Deferral does not declare the competing block invalid.
    Indexing,
}

/// Distinguishes invalid blocks from blocks waiting for local execution data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockExecutionOutcome {
    Accepted,
    Deferred(ExecutionDeferral),
    Rejected,
}

/// Chain execution operations required by FCM.
#[async_trait]
pub trait ChainController: Send + Sync {
    /// Returns a protocol verdict or an explicit retry reason.
    ///
    /// Errors indicate local failures that require service termination without rejecting the block.
    async fn try_exec_block(
        &self,
        block: OLBlockCommitment,
    ) -> anyhow::Result<BlockExecutionOutcome>;
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
    /// Reads a bounded status page to refill the in-memory retry queue.
    ///
    /// Reads only slot/status metadata after `start`, in slot then block-ID order.
    /// The lower bound is exclusive and may refer to a deleted block.
    async fn scan_block_statuses(
        &self,
        start: StatusScanStart,
        limit: NonZeroUsize,
    ) -> DbResult<Vec<(OLBlockCommitment, BlockStatus)>>;

    async fn set_block_status(&self, blkid: OLBlockId, status: BlockStatus) -> DbResult<bool>;

    /// Returns whether this invalid block needs no further rejection cleanup.
    async fn is_block_rejection_complete(&self, blkid: OLBlockId) -> DbResult<bool>;

    /// Reads at most `limit` status rows in block-ID order, independently of finality.
    ///
    /// The boolean marks unfinished rejection cleanup. Includes nonmatching rows to advance
    /// the exclusive `after` cursor without scanning an unbounded amount of history.
    /// Treats unreadable completion markers as pending so cleanup retries those reads
    /// without blocking discovery of subsequent rows.
    async fn scan_block_rejection_cleanup(
        &self,
        after: Option<OLBlockId>,
        limit: usize,
    ) -> DbResult<Vec<(OLBlockId, bool)>>;

    /// Records completion after every rejection cleanup operation succeeds.
    async fn mark_block_rejection_complete(&self, block: OLBlockCommitment) -> DbResult<()>;

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
    ) -> DbResult<Option<Arc<OLStateV1>>>;

    async fn get_canonical_block_at(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>>;

    /// Replaces the canonical suffix from `start_slot` with `block_ids`.
    async fn replace_canonical_suffix_from(
        &self,
        start_slot: Slot,
        block_ids: Vec<OLBlockId>,
    ) -> DbResult<()>;

    async fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>>;
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
pub trait FcmContext:
    ChainController + CsmStatusReader + FcmStorage + FcmStartupReconciler + Send + Sync + 'static
{
    fn publish_sync_status(&self, status: OLSyncStatus);
}
