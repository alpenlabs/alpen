//! Checkpoint-proof database interface.

use serde::{Deserialize, Serialize};
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::EpochCommitment;

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// An encoded proof receipt, stored as an opaque payload.
///
/// The database deliberately does not interpret these bytes. Proof receipts are a proving-system
/// concern, so the concrete receipt type and its encoding live with the prover-side consumers;
/// storage only needs to round-trip the payload. This keeps zkvm types out of the database layer
/// entirely.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofReceiptEntry(Vec<u8>);

impl ProofReceiptEntry {
    /// Wraps already-encoded receipt bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Returns the encoded receipt bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the entry and returns the encoded receipt bytes.
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

impl AsRef<[u8]> for ProofReceiptEntry {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for ProofReceiptEntry {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

/// Checkpoint-proof storage.
///
/// Keyed by [`EpochCommitment`] — the commitment whose checkpoint this
/// proof attests to. Each proof kind has its own peer trait + manager
/// (no shared enum). Future EE chunk / EE acct proofs will be
/// `EeChunkProofDatabase`, `EeAcctProofDatabase`, etc.
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
    fn put_proof(&self, epoch: EpochCommitment, proof: ProofReceiptEntry) -> DbResult<()>;

    /// Retrieves the checkpoint proof for the given epoch.
    ///
    /// Returns `Some(proof)` if found, or `None` if not.
    fn get_proof(&self, epoch: EpochCommitment) -> DbResult<Option<ProofReceiptEntry>>;

    /// Deletes the checkpoint proof for the given epoch.
    ///
    /// Tries to delete the proof, returning whether it really existed.
    fn del_proof(&self, epoch: EpochCommitment) -> DbResult<bool>;
}
