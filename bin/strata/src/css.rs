//! Checkpoint sync service wiring for the Strata binary.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use strata_chain_worker_new::ChainWorkerHandle;
use strata_checkpoint_types::EpochSummary;
use strata_consensus_logic::checkpoint_sync::{
    CheckpointSyncCtx, CssServiceHandle, start_css_service,
};
use strata_csm_types::CheckpointL1Ref;
use strata_csm_worker::CsmWorkerStatus;
use strata_db_types::DbResult;
use strata_identifiers::Epoch;
use strata_node_context::NodeContext;
use strata_params::RollupParams;
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
    /// Rollup params, used for the L1 reorg-safe depth.
    rollup_params: RollupParams,
}

impl StrataCheckpointSyncContext {
    fn new(
        storage: Arc<NodeStorage>,
        chain_worker: Arc<ChainWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        status_channel: Arc<StatusChannel>,
        rollup_params: RollupParams,
    ) -> Self {
        Self {
            storage,
            chain_worker,
            csm_monitor,
            status_channel,
            rollup_params,
        }
    }
}

impl CheckpointSyncCtx for StrataCheckpointSyncContext {
    fn rollup_params(&self) -> &RollupParams {
        &self.rollup_params
    }

    async fn fetch_l1_tip_height(&self) -> anyhow::Result<L1Height> {
        let tip = self
            .storage
            .l1()
            .get_canonical_chain_tip_async()
            .await?
            .ok_or_else(|| anyhow!("no L1 canonical chain tip in db"))?;
        Ok(tip.0)
    }

    async fn fetch_csm_status(&self) -> anyhow::Result<CsmWorkerStatus> {
        Ok(self.csm_monitor.get_current())
    }

    async fn get_canonical_epoch_commitment(&self, ep: Epoch) -> DbResult<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_async(ep)
            .await
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

    async fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>> {
        self.storage
            .ol_checkpoint()
            .get_epoch_summary_async(epoch)
            .await
    }

    async fn apply_checkpoint(&self, epoch: EpochCommitment) -> anyhow::Result<()> {
        Ok(self.chain_worker.apply_checkpoint(epoch).await?)
    }

    async fn update_safe_tip(&self, tip: OLBlockCommitment) -> anyhow::Result<()> {
        Ok(self.chain_worker.update_safe_tip(tip).await?)
    }

    async fn finalize_epoch(&self, epoch: EpochCommitment) -> anyhow::Result<()> {
        Ok(self.chain_worker.finalize_epoch(epoch).await?)
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
        nodectx.params().rollup().clone(),
    ));

    nodectx.task_manager().handle().block_on(start_css_service(
        css_ctx,
        checkpoint_state_rx,
        nodectx.executor().clone(),
    ))
}
