//! Dependency context for the checkpoint sync service.

use std::future::Future;

use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_csm_worker::CsmWorkerStatus;
use strata_db_types::DbResult;
use strata_identifiers::Epoch;
use strata_primitives::{EpochCommitment, L1Height, OLBlockCommitment};
use strata_status::OLSyncStatus;

use crate::checkpoint_sync::errors::CheckpointSyncResult;

/// Operations the checkpoint sync service needs from its environment.
///
/// The concrete implementation is assembled in the binary, keeping this module
/// free of any dependency on `NodeContext`.
pub trait CheckpointSyncCtx: Send + Sync + 'static {
    /// Returns the L1 reorg-safe depth, the number of confirmations after which
    /// an L1 block is considered safe from reorgs.
    fn l1_reorg_safe_depth(&self) -> u32;

    /// Fetches the current L1 chain tip height, if any. Returns `None` if L1 info is not
    /// available.
    fn fetch_l1_tip_height(
        &self,
    ) -> impl Future<Output = CheckpointSyncResult<Option<L1Height>>> + Send;

    /// Fetches the current CSM worker status.
    fn fetch_csm_status(
        &self,
    ) -> impl Future<Output = CheckpointSyncResult<CsmWorkerStatus>> + Send;

    /// Gets the canonical epoch commitment for a given epoch number.
    fn get_canonical_epoch_commitment(
        &self,
        ep: Epoch,
    ) -> impl Future<Output = DbResult<Option<EpochCommitment>>> + Send;

    /// Gets the L1 reference of a checkpoint for the given epoch, if present.
    fn get_checkpoint_l1_ref(
        &self,
        epoch: EpochCommitment,
    ) -> impl Future<Output = DbResult<Option<CheckpointL1Ref>>> + Send;

    /// Gets the epoch summary for the given epoch, if present.
    fn get_epoch_summary(
        &self,
        epoch: EpochCommitment,
    ) -> impl Future<Output = DbResult<Option<EpochSummary>>> + Send;

    /// Reconstructs and persists an epoch's OL state from its checkpoint via the
    /// chain worker.
    fn apply_checkpoint(
        &self,
        epoch: EpochCommitment,
    ) -> impl Future<Output = CheckpointSyncResult<()>> + Send;

    /// Updates the chain worker's safe tip.
    fn update_safe_tip(
        &self,
        tip: OLBlockCommitment,
    ) -> impl Future<Output = CheckpointSyncResult<()>> + Send;

    /// Finalizes an epoch in the chain worker.
    fn finalize_epoch(
        &self,
        epoch: EpochCommitment,
    ) -> impl Future<Output = CheckpointSyncResult<()>> + Send;

    /// Publishes an OL sync status update to the status channel.
    fn publish_ol_sync_status(&self, status: OLSyncStatus);
}
