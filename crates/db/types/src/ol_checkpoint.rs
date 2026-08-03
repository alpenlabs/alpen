//! OL checkpoint database interface.

use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::{Epoch, EpochCommitment};

use crate::common::L1PayloadIntentIndex;
#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Database for OL checkpoint data.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:ol_checkpoint")
)]
pub trait OLCheckpointDatabase: Send + Sync + 'static {
    /// Inserts an epoch summary retrievable by its epoch commitment.
    ///
    /// Fails if there's already an entry there.
    fn insert_epoch_summary(&self, epoch: EpochSummary) -> DbResult<()>;

    /// Gets an epoch summary given an epoch commitment.
    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>>;

    /// Gets all commitments for an epoch. This makes no guarantees about ordering.
    fn get_epoch_commitments_at(&self, epoch: Epoch) -> DbResult<Vec<EpochCommitment>>;

    /// Gets the index of the last epoch that we have a summary for, if any.
    fn get_last_summarized_epoch(&self) -> DbResult<Option<Epoch>>;

    /// Delete a specific epoch summary by epoch commitment.
    ///
    /// Returns true if the epoch summary existed and was deleted, false otherwise.
    fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete epoch summaries from the specified epoch onwards (inclusive).
    ///
    /// This method deletes all epoch summaries with epoch index >= start_epoch.
    /// Returns a vector of deleted epoch commitments.
    fn del_epoch_summaries_from_epoch(&self, start_epoch: Epoch) -> DbResult<Vec<EpochCommitment>>;

    /// Store an OL checkpoint payload entry by epoch commitment.
    fn put_checkpoint_payload_entry(
        &self,
        epoch: EpochCommitment,
        payload: CheckpointPayload,
    ) -> DbResult<()>;

    /// Get an OL checkpoint payload entry by epoch commitment.
    fn get_checkpoint_payload_entry(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>>;

    /// Get last written checkpoint payload commitment.
    fn get_last_checkpoint_payload_epoch(&self) -> DbResult<Option<EpochCommitment>>;

    /// Delete a checkpoint payload entry by epoch commitment.
    ///
    /// Returns true if it existed and was deleted.
    /// If present, the signing entry for the same commitment is also deleted.
    fn del_checkpoint_payload_entry(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete checkpoint payload entries from the specified epoch onwards (inclusive).
    ///
    /// Returns a vector of deleted epoch commitments.
    /// Signing entries for deleted payload commitments are also deleted.
    fn del_checkpoint_payload_entries_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;

    /// Delete locally-built checkpoint payload entries from the specified epoch onwards.
    ///
    /// Returns a vector of deleted epoch commitments. Signing entries for deleted
    /// payload commitments are also deleted. L1-observed checkpoint payloads and
    /// L1 refs are preserved.
    fn del_local_checkpoint_payload_entries_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;

    /// Store an OL checkpoint signing entry by epoch.
    fn put_checkpoint_signing_entry(
        &self,
        epoch: EpochCommitment,
        payload_intent_idx: L1PayloadIntentIndex,
    ) -> DbResult<()>;

    /// Get an OL checkpoint signing entry by epoch.
    fn get_checkpoint_signing_entry(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<L1PayloadIntentIndex>>;

    /// Delete an OL checkpoint signing entry by epoch.
    ///
    /// Returns true if it existed and was deleted.
    fn del_checkpoint_signing_entry(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete checkpoint signing entries from the specified epoch onwards (inclusive).
    ///
    /// Returns a vector of deleted epoch commitments.
    fn del_checkpoint_signing_entries_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;

    /// Get the next checkpoint epoch that is unsigned.
    fn get_next_unsigned_checkpoint_epoch(&self) -> DbResult<Option<Epoch>>;

    /// Store an OL checkpoint L1 ref by epoch commitment.
    fn put_checkpoint_l1_ref(
        &self,
        epoch: EpochCommitment,
        l1_ref: CheckpointL1Ref,
    ) -> DbResult<()>;

    /// Get an OL checkpoint L1 ref by epoch commitment.
    fn get_checkpoint_l1_ref(&self, epoch: EpochCommitment) -> DbResult<Option<CheckpointL1Ref>>;

    /// Get the highest epoch commitment that has an L1 ref.
    fn get_last_checkpoint_l1_ref_epoch(&self) -> DbResult<Option<EpochCommitment>>;

    /// Get all observed `(epoch commitment, L1 ref)` pairs at or above
    /// `start_epoch`, ordered by ascending epoch.
    fn get_checkpoint_l1_refs_from(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<(EpochCommitment, CheckpointL1Ref)>>;

    /// Get the observed checkpoint commitments for `epoch`.
    ///
    /// The returned candidates are observed, not canonical: an L1 reorg can
    /// leave more than one, so callers resolve canonicity at read time.
    fn get_observed_checkpoint_commitments_for_epoch(
        &self,
        epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;

    /// Delete an OL checkpoint L1 ref by epoch commitment.
    ///
    /// Returns true if it existed and was deleted.
    fn del_checkpoint_l1_ref(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete checkpoint L1 refs from the specified epoch onwards (inclusive).
    ///
    /// Returns a vector of deleted epoch commitments.
    fn del_checkpoint_l1_refs_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;

    /// Atomically inserts the L1-observed checkpoint payload and the L1 ref
    /// for `commitment`.
    ///
    /// The payload is stored in a separate table from the sequencer's
    /// locally-built payloads so the two sources of truth stay distinct.
    /// Overwrites any existing entries.
    fn put_checkpoint_l1_observation(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
        l1_ref: CheckpointL1Ref,
    ) -> DbResult<()>;

    /// Get the L1-observed checkpoint payload by epoch commitment.
    fn get_checkpoint_l1_observed_payload(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>>;

    /// Delete the L1-observed checkpoint payload by epoch commitment.
    ///
    /// Returns true if it existed and was deleted.
    fn del_checkpoint_l1_observed_payload(&self, epoch: EpochCommitment) -> DbResult<bool>;

    /// Delete L1-observed checkpoint payloads from the specified epoch onwards
    /// (inclusive). Returns a vector of deleted epoch commitments.
    fn del_checkpoint_l1_observed_payloads_from_epoch(
        &self,
        start_epoch: Epoch,
    ) -> DbResult<Vec<EpochCommitment>>;
}
