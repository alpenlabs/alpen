//! Checkpoint sync service wiring for the Strata binary.

use std::sync::Arc;

use anyhow::Result;
use strata_chain_worker_new::ChainWorkerHandle;
use strata_checkpoint_types::EpochSummary;
use strata_consensus_logic::checkpoint_sync::{
    CheckpointSyncCtx, CheckpointSyncError, CheckpointSyncResult, CssServiceHandle, start_css,
};
use strata_csm_types::CheckpointL1Ref;
use strata_csm_worker::CsmWorkerStatus;
use strata_db_types::DbResult;
use strata_identifiers::Epoch;
use strata_node_context::NodeContext;
use strata_primitives::{EpochCommitment, L1Height, OLBlockCommitment};
use strata_service::ServiceMonitor;
use strata_status::{OLSyncStatus, OLSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;

/// Production [`CheckpointSyncCtx`] backed by node storage, the chain worker,
/// the CSM monitor and the status channel.
struct StrataCheckpointSyncContext {
    /// Node storage for checkpoint and L1 lookups.
    storage: Arc<NodeStorage>,
    /// Chain worker handle used to apply, advance and finalize epochs.
    chain_worker: Arc<ChainWorkerHandle>,
    /// Monitor exposing the current CSM worker status.
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    /// Status channel for publishing OL sync status updates.
    status_channel: Arc<StatusChannel>,
    /// L1 reorg-safe depth, after which an L1 block is considered safe from reorgs.
    l1_reorg_safe_depth: u32,
}

impl StrataCheckpointSyncContext {
    fn new(
        storage: Arc<NodeStorage>,
        chain_worker: Arc<ChainWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        status_channel: Arc<StatusChannel>,
        l1_reorg_safe_depth: u32,
    ) -> Self {
        Self {
            storage,
            chain_worker,
            csm_monitor,
            status_channel,
            l1_reorg_safe_depth,
        }
    }
}

impl CheckpointSyncCtx for StrataCheckpointSyncContext {
    fn l1_reorg_safe_depth(&self) -> u32 {
        self.l1_reorg_safe_depth
    }

    async fn fetch_l1_tip_height(&self) -> CheckpointSyncResult<Option<L1Height>> {
        let tip = self.storage.l1().get_canonical_chain_tip_async().await?;
        Ok(tip.map(|t| t.0))
    }

    async fn fetch_csm_status(&self) -> CheckpointSyncResult<CsmWorkerStatus> {
        Ok(self.csm_monitor.get_current())
    }

    async fn get_checkpoint_l1_ref(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointL1Ref>> {
        self.storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_async(epoch)
            .await
    }

    async fn get_observed_checkpoint_for_epoch(
        &self,
        ep: Epoch,
    ) -> CheckpointSyncResult<Option<EpochCommitment>> {
        let ol_checkpoint = self.storage.ol_checkpoint();
        let l1 = self.storage.l1();

        let mut canonical: Option<EpochCommitment> = None;
        for commitment in ol_checkpoint
            .get_observed_checkpoint_commitments_for_epoch_async(ep)
            .await?
        {
            let Some(l1_ref) = ol_checkpoint.get_checkpoint_l1_ref_async(commitment).await? else {
                continue;
            };

            // Drop observations recorded on an orphaned L1 block, matching CSM's
            // read-time filtering; a reorg can leave stale observations behind.
            let l1_block = l1_ref.l1_commitment;
            if l1
                .get_canonical_blockid_at_height_async(l1_block.height())
                .await?
                != Some(*l1_block.blkid())
            {
                continue;
            }

            if canonical.is_some() {
                return Err(CheckpointSyncError::AmbiguousObservation(ep));
            }
            canonical = Some(commitment);
        }

        Ok(canonical)
    }

    async fn get_genesis_epoch_commitment(&self) -> DbResult<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_async(0)
            .await
    }

    async fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>> {
        self.storage
            .ol_checkpoint()
            .get_epoch_summary_async(epoch)
            .await
    }

    async fn apply_checkpoint(&self, epoch: EpochCommitment) -> CheckpointSyncResult<()> {
        self.chain_worker
            .apply_checkpoint(epoch)
            .await
            .map_err(|cause| CheckpointSyncError::EpochOp {
                epoch,
                op: "apply_checkpoint",
                cause,
            })
    }

    async fn update_safe_tip(&self, tip: OLBlockCommitment) -> CheckpointSyncResult<()> {
        self.chain_worker
            .update_safe_tip(tip)
            .await
            .map_err(CheckpointSyncError::SafeTipUpdate)
    }

    async fn finalize_epoch(&self, epoch: EpochCommitment) -> CheckpointSyncResult<()> {
        self.chain_worker
            .finalize_epoch(epoch)
            .await
            .map_err(|cause| CheckpointSyncError::EpochOp {
                epoch,
                op: "finalize_epoch",
                cause,
            })
    }

    fn publish_ol_sync_status(&self, status: OLSyncStatus) {
        self.status_channel
            .update_ol_sync_status(OLSyncStatusUpdate::new(status));
    }
}

/// Starts the checkpoint sync service.
pub(crate) fn start(
    nodectx: &NodeContext,
    chain_worker_handle: Arc<ChainWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
) -> Result<CssServiceHandle> {
    let checkpoint_state_rx = nodectx.status_channel().subscribe_checkpoint_state();
    let css_ctx = Arc::new(StrataCheckpointSyncContext::new(
        nodectx.storage().clone(),
        chain_worker_handle,
        csm_monitor,
        nodectx.status_channel().clone(),
        nodectx.config().btcio.l1_reorg_safe_depth,
    ));

    nodectx.task_manager().handle().block_on(start_css(
        css_ctx,
        checkpoint_state_rx,
        nodectx.executor().clone(),
    ))
}
