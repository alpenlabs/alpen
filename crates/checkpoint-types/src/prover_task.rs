//! Task-key wrapper used by the integrated checkpoint prover.
//!
//! Lives in a shared crate so the running node (`bin/strata`) and offline
//! admin tooling (`bin/strata-dbtool`) agree on the on-disk byte format
//! for entries in the [`strata_db_types::traits::ProverTaskDatabase`].
//!
//! Wire format is `borsh::to_vec(&CheckpointProofTask(commitment))`.
//! Because borsh serializes a tuple newtype as its inner field, the
//! resulting bytes are identical to `borsh::to_vec(&commitment)` — but
//! the explicit wrapper documents the contract and gives both sides a
//! single import point.

use std::fmt;

use strata_identifiers::EpochCommitment;

/// Task identifier for an integrated checkpoint proof.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CheckpointProofTask(pub EpochCommitment);

impl CheckpointProofTask {
    /// Returns the underlying epoch commitment.
    pub fn commitment(&self) -> EpochCommitment {
        self.0
    }
}

impl fmt::Display for CheckpointProofTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
