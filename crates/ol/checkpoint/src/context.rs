//! Context trait for checkpoint worker dependencies.

use std::sync::Arc;

use strata_checkpoint_types::EpochSummary;
use strata_db_types::types::OLCheckpointEntry;
use strata_identifiers::Epoch;
use strata_ol_chain_types_new::OLLog;
use strata_primitives::epoch::EpochCommitment;
use strata_storage::NodeStorage;

/// Context providing dependencies for the checkpoint worker.
///
/// This trait abstracts storage and data providers, enabling testing
/// with mock implementations and future production providers.
pub(crate) trait CheckpointWorkerContext: Send + Sync + 'static {
    /// Get the last summarized epoch index, if any.
    fn get_last_summarized_epoch(&self) -> anyhow::Result<Option<u64>>;

    /// Get the canonical epoch commitment for a given epoch index.
    fn get_canonical_epoch_commitment_at(
        &self,
        index: u64,
    ) -> anyhow::Result<Option<EpochCommitment>>;

    /// Get the epoch summary for a commitment.
    fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<EpochSummary>>;

    /// Get a checkpoint entry for the given epoch.
    fn get_checkpoint(&self, epoch: Epoch) -> anyhow::Result<Option<OLCheckpointEntry>>;

    /// Get the last checkpointed epoch, if any.
    fn get_last_checkpoint_epoch(&self) -> anyhow::Result<Option<Epoch>>;

    /// Store a checkpoint entry for the given epoch.
    fn put_checkpoint(&self, epoch: Epoch, entry: OLCheckpointEntry) -> anyhow::Result<()>;

    /// Gets aggregated OL logs for the epoch.
    fn get_epoch_logs(&self, epoch: &EpochCommitment) -> anyhow::Result<Vec<OLLog>>;

    /// Gets proof bytes for the checkpoint.
    fn get_proof(&self, epoch: &EpochCommitment) -> anyhow::Result<Vec<u8>>;
}

/// Production context implementation with v1 defaults.
///
/// Uses empty DA, empty logs, and placeholder proof.
pub(crate) struct CheckpointWorkerContextImpl {
    storage: Arc<NodeStorage>,
}

impl CheckpointWorkerContextImpl {
    /// Create a new context with the given storage.
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }
}

impl CheckpointWorkerContext for CheckpointWorkerContextImpl {
    fn get_last_summarized_epoch(&self) -> anyhow::Result<Option<u64>> {
        self.storage
            .ol_checkpoint()
            .get_last_summarized_epoch_blocking()
            .map_err(Into::into)
    }

    fn get_canonical_epoch_commitment_at(
        &self,
        index: u64,
    ) -> anyhow::Result<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_blocking(index)
            .map_err(Into::into)
    }

    fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<EpochSummary>> {
        self.storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(commitment)
            .map_err(Into::into)
    }

    fn get_checkpoint(&self, epoch: Epoch) -> anyhow::Result<Option<OLCheckpointEntry>> {
        self.storage
            .ol_checkpoint()
            .get_checkpoint_blocking(epoch)
            .map_err(Into::into)
    }

    fn get_last_checkpoint_epoch(&self) -> anyhow::Result<Option<Epoch>> {
        self.storage
            .ol_checkpoint()
            .get_last_checkpoint_epoch_blocking()
            .map_err(Into::into)
    }

    fn put_checkpoint(&self, epoch: Epoch, entry: OLCheckpointEntry) -> anyhow::Result<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_blocking(epoch, entry)
            .map_err(Into::into)
    }

    fn get_epoch_logs(&self, _epoch: &EpochCommitment) -> anyhow::Result<Vec<OLLog>> {
        // V1: empty logs
        Ok(Vec::new())
    }

    fn get_proof(&self, _epoch: &EpochCommitment) -> anyhow::Result<Vec<u8>> {
        // V1: empty placeholder proof
        Ok(Vec::new())
    }
}
