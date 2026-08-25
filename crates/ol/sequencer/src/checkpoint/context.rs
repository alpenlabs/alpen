//! Infrastructure capabilities used by checkpoint publication and recovery.

use async_trait::async_trait;
use strata_db_types::l1_broadcast::L1TxEntry;
use strata_db_types::l1_writer::{BundledPayloadEntry, IntentEntry};
use strata_db_types::DbError;
use strata_identifiers::{Buf32, Epoch, EpochCommitment};

/// Error boundary for checkpoint publication and reconciliation infrastructure.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointContextError {
    #[error("database operation failed: {0}")]
    Database(#[from] DbError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result returned by checkpoint context operations.
pub type CheckpointContextResult<T> = Result<T, CheckpointContextError>;

/// Provides the state needed to decide whether a checkpoint transaction may be published.
#[async_trait]
pub trait CheckpointPublishContext: Send + Sync + 'static {
    /// Returns the checkpoint epoch accepted by canonical ASM state.
    async fn get_accepted_checkpoint_epoch(&self) -> CheckpointContextResult<Option<Epoch>>;

    /// Returns the latest reorg-safe checkpoint epoch.
    async fn get_safe_checkpoint_epoch(&self) -> CheckpointContextResult<Option<Epoch>>;

    /// Returns the next broadcaster index.
    fn get_next_broadcast_idx(&self) -> CheckpointContextResult<u64>;

    /// Returns the broadcaster entry at an index.
    fn get_broadcast_entry(&self, idx: u64) -> CheckpointContextResult<Option<L1TxEntry>>;

    /// Returns the broadcaster entry for a transaction ID.
    fn get_broadcast_entry_by_id(&self, txid: Buf32) -> CheckpointContextResult<Option<L1TxEntry>>;
}

/// Provides the storage operations needed to reconcile unaccepted checkpoints.
pub trait CheckpointReconcileContext: Send + Sync + 'static {
    /// Returns the first epoch after the checkpoint accepted by canonical ASM state.
    fn get_first_unaccepted_checkpoint_epoch(&self) -> CheckpointContextResult<Option<Epoch>>;

    /// Returns locally-built checkpoint commitments at or after an epoch.
    fn get_checkpoint_payload_commitments_from_epoch(
        &self,
        epoch: Epoch,
    ) -> CheckpointContextResult<Vec<EpochCommitment>>;

    /// Returns the last summarized epoch.
    fn get_last_summarized_epoch(&self) -> CheckpointContextResult<Option<Epoch>>;

    /// Returns all checkpoint commitments for an epoch.
    fn get_epoch_commitments_at(
        &self,
        epoch: Epoch,
    ) -> CheckpointContextResult<Vec<EpochCommitment>>;

    /// Returns the next writer intent index.
    fn get_next_intent_idx(&self) -> CheckpointContextResult<u64>;

    /// Returns the writer intent at an index.
    fn get_intent_by_idx(&self, idx: u64) -> CheckpointContextResult<Option<IntentEntry>>;

    /// Atomically stores an abandoned payload and marks its intent as bundled.
    fn abandon_unbundled_intent(
        &self,
        intent: IntentEntry,
        payload: BundledPayloadEntry,
    ) -> CheckpointContextResult<()>;

    /// Returns the writer payload at an index.
    fn get_payload_by_idx(&self, idx: u64) -> CheckpointContextResult<Option<BundledPayloadEntry>>;

    /// Stores a writer payload at an existing index.
    fn put_payload(&self, idx: u64, payload: BundledPayloadEntry) -> CheckpointContextResult<()>;

    /// Returns the broadcaster entry for a transaction ID.
    fn get_broadcast_entry_by_id(&self, txid: Buf32) -> CheckpointContextResult<Option<L1TxEntry>>;

    /// Stores a broadcaster entry for a transaction ID.
    fn put_broadcast_entry(&self, txid: Buf32, entry: L1TxEntry) -> CheckpointContextResult<()>;

    /// Deletes a checkpoint proof.
    fn delete_checkpoint_proof(&self, commitment: EpochCommitment)
        -> CheckpointContextResult<bool>;

    /// Deletes a checkpoint prover task.
    fn delete_checkpoint_prover_task(
        &self,
        commitment: EpochCommitment,
    ) -> CheckpointContextResult<bool>;

    /// Deletes an unobserved, locally-built checkpoint payload.
    fn delete_unobserved_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> CheckpointContextResult<bool>;
}
