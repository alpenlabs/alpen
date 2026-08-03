//! Task-key wrapper used by the integrated checkpoint prover.
//!
//! Lives in a shared crate so the running node (`bin/strata`) and offline
//! admin tooling (`bin/strata-dbtool`) agree on the on-disk byte format
//! for entries in the [`strata_db_types::prover_task::ProverTaskDatabase`].
//!
//! Wire format is `borsh::to_vec(&CheckpointProofTask(commitment))`.
//! Because borsh serializes a tuple newtype as its inner field, the
//! resulting bytes are identical to `borsh::to_vec(&commitment)` — but
//! the explicit wrapper documents the contract and gives both sides a
//! single import point.

use std::fmt;

use borsh::{io::Error as BorshIoError, BorshDeserialize, BorshSerialize};
use strata_identifiers::EpochCommitment;

/// Task identifier for an integrated checkpoint proof.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize)]
pub struct CheckpointProofTask(pub EpochCommitment);

impl CheckpointProofTask {
    /// Returns the underlying epoch commitment.
    pub fn commitment(&self) -> EpochCommitment {
        self.0
    }

    /// Encodes the task into its stored byte form.
    pub fn to_key_bytes(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("CheckpointProofTask borsh-serializable")
    }
}

impl fmt::Display for CheckpointProofTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<CheckpointProofTask> for Vec<u8> {
    fn from(task: CheckpointProofTask) -> Self {
        task.to_key_bytes()
    }
}

impl TryFrom<Vec<u8>> for CheckpointProofTask {
    type Error = BorshIoError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        borsh::from_slice(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_bytes_roundtrip() {
        let task = CheckpointProofTask(EpochCommitment::null());
        let bytes = task.to_key_bytes();
        let decoded = CheckpointProofTask::try_from(bytes).unwrap();
        assert_eq!(decoded, task);
    }

    #[test]
    fn key_bytes_match_borsh_of_inner_commitment() {
        // The on-disk format is documented as `borsh::to_vec(&task)` — which,
        // because borsh serializes a tuple newtype as its inner field, must
        // equal `borsh::to_vec(&commitment)`. This invariant lets external
        // tooling reconstruct the key without depending on this wrapper.
        let commit = EpochCommitment::null();
        let task = CheckpointProofTask(commit);
        assert_eq!(task.to_key_bytes(), borsh::to_vec(&commit).unwrap());
    }

    #[test]
    fn commitment_accessor_returns_inner() {
        let commit = EpochCommitment::null();
        let task = CheckpointProofTask(commit);
        assert_eq!(task.commitment(), commit);
    }
}
