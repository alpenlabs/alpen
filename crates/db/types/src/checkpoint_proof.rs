//! Checkpoint-proof database interface.

#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::EpochCommitment;
use zkaleido::ProofReceiptWithMetadata;

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Checkpoint-proof storage.
///
/// Keyed by [`EpochCommitment`] — the commitment whose checkpoint this
/// proof attests to. Each proof kind has its own peer trait + manager
/// (no shared enum, no opaque-byte scheme). Future EE chunk / EE acct
/// proofs will be `EeChunkProofDatabase`, `EeAcctProofDatabase`, etc.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:checkpoint_proof")
)]
pub trait CheckpointProofDatabase: Send + Sync + 'static {
    /// Upserts a checkpoint proof for the given epoch.
    ///
    /// Overwrites any existing proof for the same epoch. Re-proves attest to
    /// the same statement, so overwriting is safe and keeps the receipt hook
    /// idempotent — refusing the write would surface as a spurious storage
    /// error on the prover task.
    fn put_proof(&self, epoch: EpochCommitment, proof: ProofReceiptWithMetadata) -> DbResult<()>;

    /// Retrieves the checkpoint proof for the given epoch.
    ///
    /// Returns `Some(proof)` if found, or `None` if not.
    fn get_proof(&self, epoch: EpochCommitment) -> DbResult<Option<ProofReceiptWithMetadata>>;

    /// Deletes the checkpoint proof for the given epoch.
    ///
    /// Tries to delete the proof, returning whether it really existed.
    fn del_proof(&self, epoch: EpochCommitment) -> DbResult<bool>;
}
