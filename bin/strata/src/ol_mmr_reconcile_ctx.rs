//! Binary-local storage context for OL MMR index reconciliation.

use async_trait::async_trait;
use strata_acct_types::Mmr64;
use strata_chain_worker::prefill_l1_block_refs_mmr;
use strata_consensus_logic::ol_mmr_reconcile::{OLMmrReconcileCtx, OLMmrReconcileTarget};
use strata_db_types::{DbResult, MmrId, RawMmrId};
use strata_ol_params::OLParams;
use strata_storage::NodeStorage;

/// Supplies Strata binary storage dependencies to the OL MMR reconciler.
pub(crate) struct StrataMmrReconcileCtx<'a> {
    storage: &'a NodeStorage,
    ol_params: &'a OLParams,
}

impl<'a> StrataMmrReconcileCtx<'a> {
    /// Creates a storage-backed reconciliation context.
    pub(crate) fn new(storage: &'a NodeStorage, ol_params: &'a OLParams) -> Self {
        Self { storage, ol_params }
    }
}

#[async_trait]
impl OLMmrReconcileCtx for StrataMmrReconcileCtx<'_> {
    async fn prefill_l1_block_refs_mmr(&self) -> DbResult<()> {
        prefill_l1_block_refs_mmr(
            self.storage.mmr_index(),
            self.ol_params.last_l1_block.height() as u64,
        )
        .await
    }

    async fn list_mmr_ids(&self) -> DbResult<Vec<RawMmrId>> {
        self.storage.mmr_index().list_mmr_ids().await
    }

    async fn get_mmr_leaf_count(&self, mmr_id: &MmrId) -> DbResult<u64> {
        self.storage
            .mmr_index()
            .get_handle(mmr_id.clone())
            .get_leaf_count()
            .await
    }

    async fn get_mmr_state_at(&self, mmr_id: &MmrId, leaf_count: u64) -> DbResult<Mmr64> {
        self.storage
            .mmr_index()
            .get_handle(mmr_id.clone())
            .get_state_at(leaf_count)
            .await
    }

    async fn truncate_mmr_to_leaf_count(
        &self,
        mmr_id: &MmrId,
        target_leaf_count: u64,
    ) -> DbResult<()> {
        self.storage
            .mmr_index()
            .get_handle(mmr_id.clone())
            .truncate_to_leaf_count(target_leaf_count)
            .await
    }

    async fn reconcile_ol_state_indexing_to_target(
        &self,
        target: &OLMmrReconcileTarget,
    ) -> DbResult<()> {
        if !target.rejected_indexing_blocks.is_empty() {
            self.storage
                .ol_state_indexing()
                .del_block_attributed_indexing_async(
                    target.epoch,
                    target.rejected_indexing_blocks.clone(),
                )
                .await?;
        }

        self.storage
            .ol_state_indexing()
            .rollback_to_epoch_async(target.epoch)
            .await?;
        self.storage
            .ol_state_indexing()
            .rollback_to_block_async(target.epoch, target.block)
            .await
    }
}
