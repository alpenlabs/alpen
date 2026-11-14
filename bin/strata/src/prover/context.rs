//! Storage and proving capabilities used by the checkpoint runner.

use std::{collections::BTreeMap, future::Future, slice, sync::Arc};

use anyhow::Result;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Epoch, EpochCommitment, OLBlockCommitment, Slot};
use strata_ol_canonical_chain_provider::{OLCanonicalChainProvider, OLCanonicalChainProviderImpl};
use strata_ol_chain_types_v1::OLBlockHeaderV1;
use strata_ol_checkpoint::CanonicalEpochReconciler;
use strata_ol_state_types::OLSpecId;
use strata_paas::{ProverHandle, TaskResult};
use strata_storage::NodeStorage;
use tokio::task::spawn_blocking;

use super::{
    errors::{CheckpointHistoryError, ProverError},
    spec::{CheckpointSpec, CheckpointTask, checkpoint_task_spec, ensure_canonical_task},
};

pub(super) trait CheckpointProverContext: Send + Sync {
    /// Gets the canonical tip, or `None` before the canonical chain is initialized.
    fn get_canonical_tip(&self) -> impl Future<Output = Result<Option<OLBlockCommitment>>> + Send;

    /// Gets the stored header for `block`, or `None` if it is not stored.
    fn get_block_header(
        &self,
        block: OLBlockCommitment,
    ) -> impl Future<Output = Result<Option<OLBlockHeaderV1>>> + Send;

    /// Gets the canonical commitment for `epoch`.
    fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> impl Future<Output = Result<Option<EpochCommitment>>> + Send;

    /// Gets the canonical block at `slot`, or `None` when there is no canonical block at that slot.
    fn get_canonical_block_at(
        &self,
        slot: Slot,
    ) -> impl Future<Output = Result<Option<OLBlockCommitment>>> + Send;

    /// Finds the canonical point after which proving can resume.
    fn find_canonical_resume_point(
        &self,
        last_processed: EpochCommitment,
        history_base: Option<EpochCommitment>,
    ) -> impl Future<Output = Result<Option<EpochCommitment>>> + Send;

    /// Gets the base of retained OL history.
    fn get_history_base(&self) -> impl Future<Output = Result<Option<EpochCommitment>>> + Send;

    /// Gets the summary for exactly `commitment`; a missing or mismatched summary is an error.
    fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> impl Future<Output = Result<EpochSummary>> + Send;

    /// Gets the commitment of the latest stored checkpoint payload.
    fn get_last_checkpoint_payload_epoch(
        &self,
    ) -> impl Future<Output = Result<Option<EpochCommitment>>> + Send;

    /// Whether a checkpoint payload is stored for `commitment`.
    fn has_payload(&self, commitment: EpochCommitment)
    -> impl Future<Output = Result<bool>> + Send;

    /// Whether a proof is stored for `commitment`.
    fn has_proof(&self, commitment: EpochCommitment) -> impl Future<Output = Result<bool>> + Send;

    /// Submits idempotently and waits for the proof.
    ///
    /// Dropping the future stops waiting but leaves the saved task intact and any
    /// in-flight backend proof running.
    fn prove(&self, commitment: EpochCommitment) -> impl Future<Output = Result<()>> + Send;
}

pub(super) struct CheckpointProverContextImpl {
    storage: Arc<NodeStorage>,
    canonical_chain_provider: OLCanonicalChainProviderImpl,
    epoch_reconciler: CanonicalEpochReconciler,
    provers: BTreeMap<OLSpecId, ProverHandle<CheckpointSpec>>,
}

impl CheckpointProverContextImpl {
    pub(super) fn new(
        storage: Arc<NodeStorage>,
        provers: BTreeMap<OLSpecId, ProverHandle<CheckpointSpec>>,
    ) -> Self {
        Self {
            canonical_chain_provider: OLCanonicalChainProviderImpl::new(&storage),
            epoch_reconciler: CanonicalEpochReconciler::new(&storage),
            storage,
            provers,
        }
    }

    pub(super) async fn get_task_spec(&self, commitment: EpochCommitment) -> Result<OLSpecId> {
        ensure_canonical_task(&self.canonical_chain_provider, commitment).await?;
        let storage = Arc::clone(&self.storage);
        Ok(
            spawn_blocking(move || checkpoint_task_spec(&storage, CheckpointTask(commitment)))
                .await??,
        )
    }
}

impl CheckpointProverContext for CheckpointProverContextImpl {
    async fn get_canonical_tip(&self) -> Result<Option<OLBlockCommitment>> {
        Ok(self
            .canonical_chain_provider
            .get_canonical_tip_async()
            .await?)
    }

    async fn get_block_header(&self, block: OLBlockCommitment) -> Result<Option<OLBlockHeaderV1>> {
        Ok(self
            .storage
            .ol_block()
            .get_ol_header_async(*block.blkid())
            .await?)
    }

    async fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> Result<Option<EpochCommitment>> {
        Ok(self
            .canonical_chain_provider
            .get_canonical_epoch_commitment_at_async(epoch)
            .await?)
    }

    async fn get_canonical_block_at(&self, slot: Slot) -> Result<Option<OLBlockCommitment>> {
        Ok(self
            .canonical_chain_provider
            .get_canonical_block_at_async(slot)
            .await?)
    }

    async fn find_canonical_resume_point(
        &self,
        last_processed: EpochCommitment,
        history_base: Option<EpochCommitment>,
    ) -> Result<Option<EpochCommitment>> {
        let reconciler = self.epoch_reconciler.clone();
        Ok(spawn_blocking(move || {
            reconciler.find_canonical_resume_point(last_processed, history_base)
        })
        .await??)
    }

    async fn get_history_base(&self) -> Result<Option<EpochCommitment>> {
        Ok(self.storage.ol_block().get_history_base_async().await?)
    }

    async fn get_epoch_summary(&self, commitment: EpochCommitment) -> Result<EpochSummary> {
        let summary = self
            .storage
            .ol_checkpoint()
            .get_epoch_summary_async(commitment)
            .await?
            .ok_or(CheckpointHistoryError::MissingSummary(commitment))?;
        let actual = summary.get_epoch_commitment();
        if actual != commitment {
            return Err(CheckpointHistoryError::SummaryMismatch {
                expected: commitment,
                actual,
            }
            .into());
        }
        Ok(summary)
    }

    async fn get_last_checkpoint_payload_epoch(&self) -> Result<Option<EpochCommitment>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_last_checkpoint_payload_epoch_async()
            .await?)
    }

    async fn has_payload(&self, commitment: EpochCommitment) -> Result<bool> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_checkpoint_payload_entry_async(commitment)
            .await?
            .is_some())
    }

    async fn has_proof(&self, commitment: EpochCommitment) -> Result<bool> {
        let proofs = self.storage.checkpoint_proof().clone();
        Ok(spawn_blocking(move || proofs.get_proof(&commitment))
            .await??
            .is_some())
    }

    async fn prove(&self, commitment: EpochCommitment) -> Result<()> {
        let spec = self.get_task_spec(commitment).await?;
        let task = CheckpointTask(commitment);
        let handle = self
            .provers
            .get(&spec)
            .ok_or(MissingCheckpointArtifact(spec))?;
        handle.submit(task).await?;
        let results = handle.wait_for_tasks(slice::from_ref(&task)).await?;
        match results
            .into_iter()
            .next()
            .expect("one result for one checkpoint task")
        {
            TaskResult::Completed { .. } => Ok(()),
            TaskResult::Failed { error, .. } => {
                Err(anyhow::anyhow!("checkpoint proof failed: {error}"))
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("no checkpoint artifact is configured for OL spec {0:?}")]
pub(super) struct MissingCheckpointArtifact(OLSpecId);

pub(super) fn is_waiting_for_state(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<ProverError>(),
        Some(
            ProverError::EpochCommitmentNotFound(_)
                | ProverError::EpochTerminalStateNotFound { .. }
                | ProverError::StaleTaskCommitment { .. }
        )
    )
}
