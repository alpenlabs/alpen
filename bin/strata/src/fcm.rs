//! Fork-choice manager service wiring for the Strata binary.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use strata_chain_worker_new::ChainWorkerHandle;
use strata_consensus_logic::{
    ChainController, CsmStatusReader, FcmContext, FcmServiceHandle, FcmStorage, start_fcm_service,
    unfinalized_tracker::UnfinalizedOLBlockSource,
};
use strata_csm_worker::CsmWorkerStatus;
use strata_db_types::{DbResult, traits::BlockStatus};
use strata_identifiers::{Epoch, Slot};
use strata_node_context::NodeContext;
use strata_ol_chain_types_new::OLBlock;
use strata_ol_state_types::OLState;
use strata_primitives::{EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_service::ServiceMonitor;
use strata_status::{OLSyncStatus, OLSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;

struct StrataFcmContext {
    storage: Arc<NodeStorage>,
    chain_worker: Arc<ChainWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    status_channel: Arc<StatusChannel>,
}

impl StrataFcmContext {
    fn new(
        storage: Arc<NodeStorage>,
        chain_worker: Arc<ChainWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        status_channel: Arc<StatusChannel>,
    ) -> Self {
        Self {
            storage,
            chain_worker,
            csm_monitor,
            status_channel,
        }
    }
}

#[async_trait]
impl ChainController for StrataFcmContext {
    async fn try_exec_block(&self, block: OLBlockCommitment) -> anyhow::Result<()> {
        self.chain_worker.try_exec_block(block).await?;
        Ok(())
    }

    async fn update_safe_tip(&self, safe_tip: OLBlockCommitment) -> anyhow::Result<()> {
        self.chain_worker.update_safe_tip(safe_tip).await?;
        Ok(())
    }

    async fn finalize_epoch(&self, epoch: EpochCommitment) -> anyhow::Result<()> {
        self.chain_worker.finalize_epoch(epoch).await?;
        Ok(())
    }
}

impl CsmStatusReader for StrataFcmContext {
    fn last_finalized_epoch(&self) -> Option<EpochCommitment> {
        self.csm_monitor.get_current().last_finalized_epoch
    }

    fn last_confirmed_epoch(&self) -> Option<EpochCommitment> {
        self.csm_monitor.get_current().last_confirmed_epoch
    }
}

#[async_trait]
impl UnfinalizedOLBlockSource for StrataFcmContext {
    async fn get_blocks_at_height(&self, slot: Slot) -> DbResult<Vec<OLBlockId>> {
        self.storage
            .ol_block()
            .get_blocks_at_height_async(slot)
            .await
    }

    async fn get_block_status(&self, blkid: OLBlockId) -> DbResult<Option<BlockStatus>> {
        self.storage.ol_block().get_block_status_async(blkid).await
    }

    async fn get_ol_block(&self, blkid: OLBlockId) -> DbResult<Option<OLBlock>> {
        self.storage.ol_block().get_block_data_async(blkid).await
    }
}

#[async_trait]
impl FcmStorage for StrataFcmContext {
    async fn set_block_status(&self, blkid: OLBlockId, status: BlockStatus) -> DbResult<bool> {
        self.storage
            .ol_block()
            .set_block_status_async(blkid, status)
            .await
    }

    async fn clear_block_high_watermark(&self, expected: OLBlockCommitment) -> DbResult<bool> {
        self.storage
            .ol_block()
            .clear_block_high_watermark_async(expected)
            .await
    }

    async fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>> {
        self.storage
            .ol_block()
            .get_block_high_watermark_async()
            .await
    }

    async fn rollback_block_state_indexing(
        &self,
        epoch: Epoch,
        cutoff: OLBlockCommitment,
    ) -> DbResult<()> {
        self.storage
            .ol_state_indexing()
            .rollback_to_block_async(epoch, cutoff)
            .await
    }

    async fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.storage
            .ol_checkpoint()
            .del_epoch_summary_async(epoch)
            .await
    }

    async fn get_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<Arc<OLState>>> {
        self.storage
            .ol_state()
            .get_toplevel_ol_state_async(commitment)
            .await
    }

    async fn get_canonical_block_at(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>> {
        self.storage
            .ol_block()
            .get_canonical_block_at_async(slot)
            .await
    }

    async fn replace_canonical_suffix_from(
        &self,
        start_slot: Slot,
        block_ids: Vec<OLBlockId>,
    ) -> DbResult<()> {
        self.storage
            .ol_block()
            .replace_canonical_suffix_from_async(start_slot, block_ids)
            .await
    }

    async fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_async(epoch)
            .await
    }
}

impl FcmContext for StrataFcmContext {
    fn publish_sync_status(&self, status: OLSyncStatus) {
        self.status_channel
            .update_ol_sync_status(OLSyncStatusUpdate::new(status));
    }
}

/// Starts the fork-choice manager service.
pub(crate) fn start(
    nodectx: &NodeContext,
    chain_worker_handle: Arc<ChainWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
) -> Result<FcmServiceHandle> {
    let checkpoint_state_rx = nodectx.status_channel().subscribe_checkpoint_state();
    let sequencer_predicate = nodectx
        .asm_params()
        .checkpoint_config()
        .ok_or_else(|| anyhow!("ASM checkpoint config required for FCM"))?
        .sequencer_predicate
        .clone();
    let fcm_ctx = Arc::new(StrataFcmContext::new(
        nodectx.storage().clone(),
        chain_worker_handle,
        csm_monitor,
        nodectx.status_channel().clone(),
    ));

    nodectx.task_manager().handle().block_on(start_fcm_service(
        sequencer_predicate,
        fcm_ctx,
        checkpoint_state_rx,
        nodectx.executor().clone(),
    ))
}
