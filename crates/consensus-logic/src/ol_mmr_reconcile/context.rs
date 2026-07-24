use async_trait::async_trait;
use strata_acct_types::Mmr64;
use strata_db_types::{DbResult, MmrId, RawMmrId};

use super::target::OLMmrReconcileTarget;

/// Storage and target lookups required by OL MMR index reconciliation.
#[async_trait]
pub trait OLMmrReconcileCtx: Send + Sync {
    /// Ensures the L1 block refs MMR contains its genesis sentinel leaves.
    async fn prefill_l1_block_refs_mmr(&self) -> DbResult<()>;

    /// Lists persisted MMR namespace ids.
    async fn list_mmr_ids(&self) -> DbResult<Vec<RawMmrId>>;

    /// Reads the persisted leaf count for an MMR namespace.
    async fn get_mmr_leaf_count(&self, mmr_id: &MmrId) -> DbResult<u64>;

    /// Reads the persisted MMR state at `leaf_count`.
    async fn get_mmr_state_at(&self, mmr_id: &MmrId, leaf_count: u64) -> DbResult<Mmr64>;

    /// Truncates an MMR namespace to `target_leaf_count`.
    async fn truncate_mmr_to_leaf_count(
        &self,
        mmr_id: &MmrId,
        target_leaf_count: u64,
    ) -> DbResult<()>;

    /// Reconciles OL state indexing rows to the selected target.
    async fn reconcile_ol_state_indexing_to_target(
        &self,
        target: &OLMmrReconcileTarget,
    ) -> DbResult<()>;
}
