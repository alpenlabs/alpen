//! Fork-choice manager service wiring for the Strata binary.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use strata_chain_worker::{ChainWorkerHandle, WorkerError};
use strata_consensus_logic::{
    BlockExecutionOutcome, BlockValidationOutcome, ChainController, CsmStatusReader,
    ExecutionDeferral, FcmContext, FcmServiceHandle, FcmStartupReconciler, FcmStorage,
    ol_mmr_reconcile::{
        OLMmrReconcileResult, OLMmrReconcileTarget, reconcile_ol_mmr_index_to_target,
    },
    start_fcm_service,
    unfinalized_tracker::UnfinalizedOLBlockSource,
};
use strata_csm_worker::CsmWorkerStatus;
use strata_db_types::{DbError, DbResult, ol_block::BlockStatus};
use strata_identifiers::{Epoch, Slot};
use strata_node_context::NodeContext;
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use strata_ol_params::OLParams;
use strata_ol_state_types::ExecError;
use strata_ol_state_types_v1::OLStateV1;
use strata_primitives::{EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_service::ServiceMonitor;
use strata_status::{OLSyncStatus, OLSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;
use tracing::{debug, warn};

use crate::ol_mmr_reconcile_ctx::StrataMmrReconcileCtx;

struct StrataFcmContext {
    storage: Arc<NodeStorage>,
    ol_params: Arc<OLParams>,
    chain_worker: Arc<ChainWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    status_channel: Arc<StatusChannel>,
}

enum WorkerFailureOutcome {
    Deferred(ExecutionDeferral),
    Rejected(ExecError),
}

fn classify_worker_failure(
    block: OLBlockCommitment,
    error: WorkerError,
) -> anyhow::Result<WorkerFailureOutcome> {
    match error {
        WorkerError::MissingPreState(_) | WorkerError::MissingOLBlock(_) => Ok(
            WorkerFailureOutcome::Deferred(ExecutionDeferral::Dependency),
        ),
        WorkerError::ManifestPending { height, reason } => {
            debug!(%block, height, ?reason, "deferring unauthenticated ASM manifest");
            Ok(WorkerFailureOutcome::Deferred(
                ExecutionDeferral::Dependency,
            ))
        }
        WorkerError::ManifestStorage(err) => {
            warn!(%block, %err, "canonical manifest storage unavailable");
            Ok(WorkerFailureOutcome::Deferred(ExecutionDeferral::Storage))
        }
        error @ WorkerError::Database(DbError::BlockIndexingConflict { .. }) => Err(error)
            .with_context(|| {
                format!(
                    "cannot execute block {block}: inconsistent local indexing; repair required"
                )
            }),
        error @ WorkerError::Database(_) => {
            warn!(%block, %error, "deferring block after local worker failure");
            Ok(WorkerFailureOutcome::Deferred(ExecutionDeferral::Storage))
        }
        WorkerError::StfExecution(error) => {
            warn!(%block, %error, "rejecting invalid block inputs");
            Ok(WorkerFailureOutcome::Rejected(error))
        }
        error => Err(error.into()),
    }
}

/// Missing durable OL inputs require repair before restored blocks can be authenticated.
fn classify_stored_input_failure(
    block: OLBlockCommitment,
    error: WorkerError,
) -> anyhow::Result<WorkerFailureOutcome> {
    match error {
        error @ (WorkerError::MissingPreState(_) | WorkerError::MissingOLBlock(_)) => {
            Err(error).with_context(|| {
                format!("cannot authenticate stored block {block}: missing durable OL data; repair required")
            })
        }
        error => classify_worker_failure(block, error),
    }
}

impl StrataFcmContext {
    fn new(
        storage: Arc<NodeStorage>,
        ol_params: Arc<OLParams>,
        chain_worker: Arc<ChainWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        status_channel: Arc<StatusChannel>,
    ) -> Self {
        Self {
            storage,
            ol_params,
            chain_worker,
            csm_monitor,
            status_channel,
        }
    }
}

#[async_trait]
impl ChainController for StrataFcmContext {
    async fn try_exec_block(
        &self,
        block: OLBlockCommitment,
    ) -> anyhow::Result<BlockExecutionOutcome> {
        match self.chain_worker.try_exec_block(block).await {
            Ok(()) => Ok(BlockExecutionOutcome::Accepted),
            Err(error) => match classify_worker_failure(block, error)? {
                WorkerFailureOutcome::Deferred(reason) => {
                    Ok(BlockExecutionOutcome::Deferred(reason))
                }
                WorkerFailureOutcome::Rejected(_) => Ok(BlockExecutionOutcome::Rejected),
            },
        }
    }

    async fn validate_block_inputs(
        &self,
        block: OLBlockCommitment,
    ) -> anyhow::Result<BlockValidationOutcome> {
        match self.chain_worker.validate_block_inputs(block).await {
            Ok(()) => Ok(BlockValidationOutcome::Authenticated),
            Err(error) => match classify_stored_input_failure(block, error)? {
                WorkerFailureOutcome::Deferred(reason) => {
                    Ok(BlockValidationOutcome::Deferred(reason))
                }
                WorkerFailureOutcome::Rejected(error) => {
                    Ok(BlockValidationOutcome::Rejected(error))
                }
            },
        }
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

    async fn get_ol_block(&self, blkid: OLBlockId) -> DbResult<Option<OLBlockV1>> {
        self.storage.ol_block().get_block_data_async(blkid).await
    }

    async fn get_ol_header(&self, blkid: OLBlockId) -> DbResult<Option<OLBlockHeaderV1>> {
        self.storage.ol_block().get_ol_header_async(blkid).await
    }
}

#[async_trait]
impl FcmStorage for StrataFcmContext {
    async fn scan_block_rejection_statuses(
        &self,
        after: Option<OLBlockId>,
        limit: usize,
    ) -> DbResult<Vec<(OLBlockId, bool)>> {
        self.storage
            .ol_block()
            .scan_block_rejection_statuses_async(after, limit)
            .await
    }

    async fn scan_block_statuses(
        &self,
        finalized_slot: Slot,
        after: Option<OLBlockCommitment>,
        limit: usize,
    ) -> DbResult<Vec<(OLBlockCommitment, BlockStatus)>> {
        self.storage
            .ol_block()
            .scan_block_statuses_async(finalized_slot, after, limit)
            .await
    }

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

    async fn is_block_rejection_complete(&self, blkid: OLBlockId) -> DbResult<bool> {
        self.storage
            .ol_block()
            .is_block_rejection_complete_async(blkid)
            .await
    }

    async fn mark_block_rejection_complete(&self, block: OLBlockCommitment) -> DbResult<()> {
        self.storage
            .ol_block()
            .mark_block_rejection_complete_async(block)
            .await
    }

    async fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>> {
        self.storage
            .ol_block()
            .get_block_high_watermark_async()
            .await
    }

    async fn get_history_base(&self) -> DbResult<Option<EpochCommitment>> {
        self.storage.ol_block().get_history_base_async().await
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
    ) -> DbResult<Option<Arc<OLStateV1>>> {
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

#[async_trait]
impl FcmStartupReconciler for StrataFcmContext {
    async fn reconcile_ol_mmr_index(
        &self,
        target: OLMmrReconcileTarget,
    ) -> OLMmrReconcileResult<()> {
        let mmr_reconcile_ctx =
            StrataMmrReconcileCtx::new(self.storage.as_ref(), self.ol_params.as_ref());

        reconcile_ol_mmr_index_to_target(&mmr_reconcile_ctx, target).await?;
        Ok(())
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
        nodectx.ol_params().clone(),
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

#[cfg(test)]
mod worker_failure_tests {
    use strata_ol_state_types::StateError;

    use super::*;

    #[test]
    fn internal_worker_failures_propagate_without_rejecting_blocks() {
        let block = OLBlockCommitment::null();
        for error in [
            WorkerError::ApplyWriteBatch {
                commitment: block,
                source: StateError::InsufficientState,
            },
            WorkerError::SnarkUpdateLogMismatch {
                expected: 1,
                found: 2,
            },
            WorkerError::MissingSummaryForEpoch(1),
            WorkerError::WorkerExited,
            WorkerError::Unexpected("injected invariant failure".to_owned()),
            WorkerError::Database(DbError::BlockIndexingConflict {
                epoch: 1,
                attempted: block,
                last_applied: block,
            }),
        ] {
            let expected = error.to_string();
            let failure = match classify_worker_failure(block, error) {
                Err(failure) => failure,
                Ok(_) => panic!("internal failures must propagate"),
            };
            assert_eq!(
                failure.downcast::<WorkerError>().unwrap().to_string(),
                expected
            );
        }
    }

    #[test]
    fn storage_failures_defer_and_protocol_failures_reject() {
        let block = OLBlockCommitment::null();
        assert!(matches!(
            classify_worker_failure(
                block,
                WorkerError::Database(DbError::Other("read failed".to_owned()))
            ),
            Ok(WorkerFailureOutcome::Deferred(ExecutionDeferral::Storage))
        ));
        assert!(matches!(
            classify_worker_failure(block, WorkerError::StfExecution(ExecError::ChainIntegrity)),
            Ok(WorkerFailureOutcome::Rejected(ExecError::ChainIntegrity))
        ));
    }
}

#[cfg(test)]
mod stored_input_failure_tests {
    use strata_chain_worker::ManifestPendingReason;

    use super::*;

    #[test]
    fn missing_durable_inputs_require_repair_only_for_stored_validation() {
        let block = OLBlockCommitment::null();
        for error in [
            WorkerError::MissingPreState(block),
            WorkerError::MissingOLBlock(*block.blkid()),
        ] {
            let failure = match classify_stored_input_failure(block, error) {
                Err(failure) => failure,
                Ok(_) => panic!("missing durable inputs must fail authentication"),
            };
            assert!(failure.to_string().contains("repair required"));
            let error = failure.downcast::<WorkerError>().unwrap();
            assert!(matches!(
                classify_worker_failure(block, error),
                Ok(WorkerFailureOutcome::Deferred(
                    ExecutionDeferral::Dependency
                ))
            ));
        }
    }

    #[test]
    fn stored_validation_still_waits_for_asm_output() {
        let block = OLBlockCommitment::null();
        assert!(matches!(
            classify_stored_input_failure(
                block,
                WorkerError::ManifestPending {
                    height: 1,
                    reason: ManifestPendingReason::MissingManifest,
                },
            ),
            Ok(WorkerFailureOutcome::Deferred(
                ExecutionDeferral::Dependency
            ))
        ));
    }
}

#[cfg(test)]
mod tests;
