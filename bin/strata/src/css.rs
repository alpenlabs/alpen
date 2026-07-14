//! Checkpoint sync service wiring for the Strata binary.

use std::{collections::BTreeSet, sync::Arc};

use anyhow::Result;
use strata_chain_worker::ChainWorkerHandle;
use strata_checkpoint_types::EpochSummary;
use strata_consensus_logic::{
    checkpoint_sync::{
        CheckpointSyncCtx, CheckpointSyncError, CheckpointSyncResult, CssServiceHandle, start_css,
    },
    ol_mmr_reconcile::{OLMmrReconcileTarget, reconcile_ol_mmr_index_to_target},
};
use strata_csm_types::CheckpointL1Ref;
use strata_csm_worker::CsmWorkerStatus;
use strata_db_types::DbResult;
use strata_identifiers::Epoch;
use strata_node_context::NodeContext;
use strata_ol_params::OLParams;
use strata_primitives::{EpochCommitment, L1Height, OLBlockCommitment};
use strata_service::ServiceMonitor;
use strata_status::{OLSyncStatus, OLSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;

use crate::ol_mmr_reconcile_ctx::StrataMmrReconcileCtx;

/// Production [`CheckpointSyncCtx`] backed by node storage, the chain worker,
/// the CSM monitor and the status channel.
struct StrataCheckpointSyncContext {
    /// Node storage for checkpoint and L1 lookups.
    storage: Arc<NodeStorage>,
    /// OL parameters used to seed MMR sentinel leaves.
    ol_params: Arc<OLParams>,
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
        ol_params: Arc<OLParams>,
        chain_worker: Arc<ChainWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        status_channel: Arc<StatusChannel>,
        l1_reorg_safe_depth: u32,
    ) -> Self {
        Self {
            storage,
            ol_params,
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

    /// Resolves the canonical checkpoint observed for `ep`.
    ///
    /// The index may hold several candidates, but only after a reorg: ASM accepts
    /// one checkpoint per epoch per chain. Keeping the one whose L1 block is still
    /// canonical leaves exactly the survivor on the current chain. Two survivors
    /// would be two checkpoints for one epoch on-chain, which can't happen, so it
    /// errors with [`CheckpointSyncError::AmbiguousObservation`].
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
            let Some(l1_ref) = ol_checkpoint
                .get_checkpoint_l1_ref_async(commitment)
                .await?
            else {
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

    async fn reconcile_ol_mmr_index(&self) -> CheckpointSyncResult<()> {
        let Some(target) = resolve_latest_summarized_epoch_mmr_target(&self.storage).await? else {
            return Ok(());
        };

        let mmr_reconcile_ctx =
            StrataMmrReconcileCtx::new(self.storage.as_ref(), self.ol_params.as_ref());
        reconcile_ol_mmr_index_to_target(&mmr_reconcile_ctx, target).await?;
        Ok(())
    }
}

/// Resolves checkpoint-sync's latest summarized epoch as an MMR reconciliation target.
#[expect(clippy::result_large_err, reason = "No need to box the service error")]
async fn resolve_latest_summarized_epoch_mmr_target(
    storage: &NodeStorage,
) -> CheckpointSyncResult<Option<OLMmrReconcileTarget>> {
    let Some(epoch) = storage
        .ol_checkpoint()
        .get_last_summarized_epoch_async()
        .await?
    else {
        return Ok(None);
    };

    let epoch_commitment = storage
        .ol_checkpoint()
        .get_canonical_epoch_commitment_at_async(epoch)
        .await?
        .ok_or(CheckpointSyncError::MissingCanonicalEpochCommitment(epoch))?;
    let summary = storage
        .ol_checkpoint()
        .get_epoch_summary_async(epoch_commitment)
        .await?
        .ok_or(CheckpointSyncError::MissingEpochSummary(epoch_commitment))?;
    let target_block = *summary.terminal();
    let target_state = storage
        .ol_state()
        .get_toplevel_ol_state_async(target_block)
        .await?
        .ok_or(CheckpointSyncError::MissingOLState(target_block))?;

    Ok(Some(OLMmrReconcileTarget::new(
        target_block,
        summary.epoch(),
        target_state,
        BTreeSet::new(),
    )))
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
        nodectx.ol_params().clone(),
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

#[cfg(test)]
mod tests {
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{Buf32, L1BlockId};
    use strata_ol_params::{BridgeParams, OLParams};
    use strata_ol_state_types::WriteBatch;
    use strata_primitives::{L1BlockCommitment, OLBlockId};
    use strata_storage::create_node_storage;

    use super::*;
    use crate::genesis::init_ol_genesis;

    fn setup_storage() -> NodeStorage {
        let db = get_test_sled_backend();
        create_node_storage(db, strata_storage::test_runtime_handle())
            .expect("test: create node storage")
    }

    fn setup_storage_with_genesis() -> (NodeStorage, OLBlockCommitment) {
        let storage = setup_storage();
        let genesis_l1_block = L1BlockCommitment::new(0, L1BlockId::from(Buf32::zero()));
        let params = OLParams::new_empty(genesis_l1_block, BridgeParams::default());
        let genesis_commitment = init_ol_genesis(&params, &storage).expect("test: init genesis");
        (storage, genesis_commitment)
    }

    #[test]
    fn test_no_mmr_target_without_summarized_epoch() {
        let storage = setup_storage();

        let target = strata_storage::test_runtime_handle()
            .block_on(resolve_latest_summarized_epoch_mmr_target(&storage))
            .expect("test: resolve target");

        assert!(target.is_none());
    }

    #[test]
    fn test_latest_summarized_epoch_sets_mmr_target() {
        let (storage, genesis_commitment) = setup_storage_with_genesis();
        let genesis_epoch_commitment = EpochCommitment::new(0, 0, *genesis_commitment.blkid());
        let genesis_summary = storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(genesis_epoch_commitment)
            .expect("test: get genesis summary")
            .expect("test: genesis summary exists");
        let genesis_state = storage
            .ol_state()
            .get_toplevel_ol_state_blocking(genesis_commitment)
            .expect("test: get genesis state")
            .expect("test: genesis state exists");
        let next_commitment = OLBlockCommitment::new(1, OLBlockId::from(Buf32::from([1u8; 32])));
        let next_summary = genesis_summary.create_next_epoch_summary(
            next_commitment,
            *genesis_summary.new_l1(),
            Buf32::from([2u8; 32]),
        );
        let mut next_state = (*genesis_state).clone();
        let mut state_writes = WriteBatch::default();
        state_writes.global_writes_mut().cur_slot = Some(next_commitment.slot());
        next_state
            .apply_write_batch(state_writes)
            .expect("test: update next state slot");
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(next_summary)
            .expect("test: insert epoch 1 summary");
        storage
            .ol_state()
            .put_toplevel_ol_state_blocking(next_commitment, next_state.clone())
            .expect("test: insert epoch 1 state");

        let target = strata_storage::test_runtime_handle()
            .block_on(resolve_latest_summarized_epoch_mmr_target(&storage))
            .expect("test: resolve target")
            .expect("test: target exists");

        assert_eq!(target.block, next_commitment);
        assert_eq!(target.epoch, next_summary.epoch());
        assert_eq!(target.state.as_ref(), &next_state);
    }

    #[test]
    fn test_mmr_target_errors_when_ol_state_is_missing() {
        let (storage, genesis_commitment) = setup_storage_with_genesis();
        storage
            .ol_state()
            .del_toplevel_ol_state_blocking(genesis_commitment)
            .expect("test: delete genesis state");

        let err = strata_storage::test_runtime_handle()
            .block_on(resolve_latest_summarized_epoch_mmr_target(&storage))
            .expect_err("test: missing state should error");

        assert!(matches!(
            err,
            CheckpointSyncError::MissingOLState(commitment) if commitment == genesis_commitment
        ));
    }
}
