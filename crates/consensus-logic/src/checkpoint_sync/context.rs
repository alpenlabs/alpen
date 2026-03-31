use strata_asm_common::AsmManifest;
use strata_chain_worker_new::ChainWorkerHandle;
use strata_checkpoint_types::EpochSummary;
use strata_db_types::DbResult;
use strata_identifiers::CheckpointL1Ref;
use strata_ol_da::{DAExtractor, ExtractedDA};
use strata_ol_state_types::OLState;
use strata_primitives::{EpochCommitment, L1Height, OLBlockCommitment};

pub trait CheckpointSyncCtx<E: DAExtractor> {
    /// Getter for chain worker handle reference.
    fn chain_worker(&self) -> &ChainWorkerHandle;

    /// Gets the corresponding epoch summary. If not found, returns error.
    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<EpochSummary>;

    /// Extract da given the extractor.
    fn extract_da_data(&self, ckpt_ref: &CheckpointL1Ref) -> anyhow::Result<ExtractedDA>;

    /// Gets state at given `OLBlockCommitment`.
    fn get_state_at(&self, blkid: OLBlockCommitment) -> anyhow::Result<OLState>;

    /// Gets asm manifets for a range.
    fn fetch_asm_manifests_range(
        &self,
        start: L1Height,
        end: L1Height,
    ) -> anyhow::Result<Vec<AsmManifest>>;
}
