use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, Buf64};
use zkaleido::Proof;

use super::batch::BatchInfo;

/// Consolidates all the information that the checkpoint is committing to, signing and proving.
#[deprecated]
#[derive(Clone, Debug, PartialEq, Eq, Arbitrary, Deserialize, Serialize)]
pub struct CheckpointCommitment {
    /// Information regarding the current batches of l1 and l2 blocks along with epoch.
    /// This is verified by the proof
    batch_info: BatchInfo,
}

/// Consolidates all information required to describe and verify a batch checkpoint.
/// This includes metadata about the batch, the state transitions, checkpoint base state,
/// and the proof itself. The proof verifies that the `transition` is valid.
#[deprecated]
#[derive(Clone, Debug, PartialEq, Eq, Arbitrary, Deserialize, Serialize)]
pub struct Checkpoint {
    /// Data that this checkpoint is committing to
    commitment: CheckpointCommitment,

    /// Proof for this checkpoint obtained from prover manager.
    proof: Proof,

    /// Additional data we post along with the checkpoint for usability.
    sidecar: CheckpointSidecar,
}

impl Checkpoint {
    pub fn new(batch_info: BatchInfo, proof: Proof, sidecar: CheckpointSidecar) -> Self {
        Self {
            commitment: CheckpointCommitment { batch_info },
            proof,
            sidecar,
        }
    }

    pub fn batch_info(&self) -> &BatchInfo {
        &self.commitment.batch_info
    }

    pub fn commitment(&self) -> &CheckpointCommitment {
        &self.commitment
    }

    pub fn proof(&self) -> &Proof {
        &self.proof
    }

    pub fn set_proof(&mut self, proof: Proof) {
        self.proof = proof
    }

    pub fn sidecar(&self) -> &CheckpointSidecar {
        &self.sidecar
    }
}

#[deprecated]
#[derive(Clone, Debug, PartialEq, Eq, Arbitrary, Deserialize, Serialize)]
pub struct CheckpointSidecar {
    /// Chainstate at the end of this checkpoint's epoch.
    /// Note: using `Vec<u8>` instead of Chainstate to avoid circular dependency with strata_state
    chainstate: Vec<u8>,
}

impl CheckpointSidecar {
    pub fn new(chainstate: Vec<u8>) -> Self {
        Self { chainstate }
    }

    pub fn chainstate(&self) -> &[u8] {
        &self.chainstate
    }
}

#[deprecated]
#[derive(Clone, Debug, Arbitrary, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedCheckpoint {
    inner: Checkpoint,
    signature: Buf64,
}

impl SignedCheckpoint {
    pub fn new(inner: Checkpoint, signature: Buf64) -> Self {
        Self { inner, signature }
    }

    pub fn checkpoint(&self) -> &Checkpoint {
        &self.inner
    }

    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }
}

impl From<SignedCheckpoint> for Checkpoint {
    fn from(value: SignedCheckpoint) -> Self {
        value.inner
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Arbitrary, Serialize, Deserialize)]
pub struct CommitmentInfo {
    pub blockhash: Buf32,
    pub txid: Buf32,
}

impl CommitmentInfo {
    pub fn new(blockhash: Buf32, txid: Buf32) -> Self {
        Self { blockhash, txid }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Arbitrary, Serialize, Deserialize)]
pub struct L1CommittedCheckpoint {
    /// The actual `Checkpoint` data.
    pub checkpoint: Checkpoint,
    /// Its commitment to L1 used to locate/identify the checkpoint in L1.
    pub commitment: CommitmentInfo,
}

impl L1CommittedCheckpoint {
    pub fn new(checkpoint: Checkpoint, commitment: CommitmentInfo) -> Self {
        Self {
            checkpoint,
            commitment,
        }
    }
}
