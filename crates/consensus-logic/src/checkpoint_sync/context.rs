use strata_chain_worker_new::ChainWorkerHandle;
use strata_checkpoint_types::EpochSummary;
use strata_db_types::DbResult;
use strata_identifiers::CheckpointL1Ref;
use strata_ol_da::{DAExtractor, OLDaPayloadV1};
use strata_primitives::EpochCommitment;

pub trait CheckpointSyncCtx<E: DAExtractor> {
    /// Getter for chain worker handle reference.
    fn chain_worker(&self) -> &ChainWorkerHandle;

    /// Gets the corresponding epoch summary. If not found, returns error.
    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<EpochSummary>;

    /// Extract da given the extractor.
    fn extract_da(&self, ckpt_ref: &CheckpointL1Ref) -> anyhow::Result<OLDaPayloadV1>;
}
