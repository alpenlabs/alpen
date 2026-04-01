use std::sync::Arc;

use anyhow::anyhow;
use strata_asm_common::AsmManifest;
use strata_chain_worker_new::ChainWorkerHandle;
use strata_checkpoint_types::EpochSummary;
use strata_db_types::DbResult;
use strata_identifiers::CheckpointL1Ref;
use strata_node_context::NodeContext;
use strata_ol_da::{DAExtractor, ExtractedDA};
use strata_ol_state_types::OLState;
use strata_primitives::{EpochCommitment, L1Height, OLBlockCommitment};
use strata_storage::NodeStorage;

pub trait CheckpointSyncCtx<E: DAExtractor> {
    /// Getter for chain worker handle reference.
    fn chain_worker(&self) -> &ChainWorkerHandle;

    /// Gets the corresponding epoch summary. If not found, returns error.
    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<EpochSummary>;

    /// Extract da given the extractor.
    fn extract_da_data(&self, ckpt_ref: &CheckpointL1Ref) -> anyhow::Result<ExtractedDA>;

    /// Gets state at given `OLBlockCommitment`.
    fn get_state_at(&self, blkid: OLBlockCommitment) -> anyhow::Result<OLState>;

    /// Gets asm manifests for a range.
    fn fetch_asm_manifests_range(
        &self,
        start: L1Height,
        end: L1Height,
    ) -> anyhow::Result<Vec<AsmManifest>>;
}

#[derive(Clone)]
#[expect(
    missing_debug_implementations,
    reason = "Not all attributes have debug impls"
)]
pub struct CheckpointSyncCtxImpl<E: DAExtractor> {
    storage: Arc<NodeStorage>,
    chain_worker: Arc<ChainWorkerHandle>,
    da_extractor: E,
}

impl<E: DAExtractor> CheckpointSyncCtxImpl<E> {
    pub fn new(
        storage: Arc<NodeStorage>,
        chain_worker: Arc<ChainWorkerHandle>,
        da_extractor: E,
    ) -> Self {
        Self {
            storage,
            chain_worker,
            da_extractor,
        }
    }
}

impl<E: DAExtractor> CheckpointSyncCtx<E> for CheckpointSyncCtxImpl<E> {
    fn chain_worker(&self) -> &ChainWorkerHandle {
        &self.chain_worker
    }

    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<EpochSummary> {
        self.storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(epoch)?
            .ok_or(strata_db_types::DbError::NonExistentEntry)
    }

    fn extract_da_data(&self, ckpt_ref: &CheckpointL1Ref) -> anyhow::Result<ExtractedDA> {
        self.da_extractor
            .extract_da(ckpt_ref)
            .map_err(|e| anyhow!("DA extraction failed: {e}"))
    }

    fn get_state_at(&self, blkid: OLBlockCommitment) -> anyhow::Result<OLState> {
        let state = self
            .storage
            .ol_state()
            .get_toplevel_ol_state_blocking(blkid)?
            .ok_or_else(|| anyhow!("missing OL state for {blkid:?}"))?;
        Ok((*state).clone())
    }

    fn fetch_asm_manifests_range(
        &self,
        start: L1Height,
        end: L1Height,
    ) -> anyhow::Result<Vec<AsmManifest>> {
        let l1_mgr = self.storage.l1();
        let mut manifests = Vec::new();
        for height in start..=end {
            let manifest = l1_mgr
                .get_block_manifest_at_height(height)?
                .ok_or_else(|| anyhow!("missing ASM manifest at L1 height {height}"))?;
            manifests.push(manifest);
        }
        Ok(manifests)
    }
}
