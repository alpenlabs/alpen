//! Binary-local storage context for OL MMR index reconciliation.

use async_trait::async_trait;
use strata_acct_types::Mmr64;
use strata_consensus_logic::ol_mmr_reconcile::{OLMmrReconcileCtx, OLMmrReconcileTarget};
use strata_db_types::{DbError, DbResult, MmrId, RawMmrId};
use strata_ol_params::OLParams;
use strata_ol_state_types::MMR_SENTINEL_DUMMY_LEAF_HASH;
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
        let target_count = self.ol_params.last_l1_block.height() as u64 + 1;
        let handle = self.storage.mmr_index().get_handle(MmrId::L1BlockRefs);
        let current_count = handle.get_leaf_count().await?;

        for expected_idx in current_count..target_count {
            let appended_idx = handle.append_leaf(MMR_SENTINEL_DUMMY_LEAF_HASH).await?;
            if appended_idx != expected_idx {
                return Err(DbError::Other(format!(
                    "L1 block refs MMR prefill index mismatch: expected {expected_idx}, got {appended_idx}"
                )));
            }
        }

        Ok(())
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
