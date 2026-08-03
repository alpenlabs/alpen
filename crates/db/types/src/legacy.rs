#![expect(deprecated, reason = "legacy old code is retained for compatibility")]
//! Deprecated checkpoint storage types retained for compatibility.
//!
//! These types are no longer referenced by any live database trait and are
//! slated for removal. New code should use the OL/EE-decoupled checkpoint
//! storage in [`crate::ol_checkpoint`].

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::Serialize;
use strata_checkpoint_types::Checkpoint;
use strata_csm_types::CheckpointL1Ref;

/// Entry corresponding to a BatchCommitment
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
#[deprecated(note = "use `OLCheckpointEntry` for OL/EE-decoupled checkpoint storage")]
pub struct CheckpointEntry {
    /// The batch checkpoint containing metadata, state transitions, and proof data.
    pub checkpoint: Checkpoint,

    /// Proving Status
    pub proving_status: CheckpointProvingStatus,

    /// Confirmation Status
    pub confirmation_status: CheckpointConfStatus,
}

impl CheckpointEntry {
    pub fn new(
        checkpoint: Checkpoint,
        proving_status: CheckpointProvingStatus,
        confirmation_status: CheckpointConfStatus,
    ) -> Self {
        Self {
            checkpoint,
            proving_status,
            confirmation_status,
        }
    }

    pub fn into_batch_checkpoint(self) -> Checkpoint {
        self.checkpoint
    }

    pub fn is_proof_ready(&self) -> bool {
        self.proving_status == CheckpointProvingStatus::ProofReady
    }
}

impl From<CheckpointEntry> for Checkpoint {
    fn from(entry: CheckpointEntry) -> Checkpoint {
        entry.into_batch_checkpoint()
    }
}

/// Status of the commmitment
#[deprecated(
    note = "use `OLCheckpointEntry::signing_status` for OL/EE-decoupled checkpoint signing status"
)]
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize)]
pub enum CheckpointProvingStatus {
    /// Proof has not been created for this checkpoint
    PendingProof,
    /// Proof is ready
    ProofReady,
}

#[deprecated(
    note = "use `OLCheckpointEntry::confirmation_status` for OL/EE-decoupled checkpoint confirmation flow"
)]
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize)]
pub enum CheckpointConfStatus {
    /// Pending to be posted on L1
    Pending,
    /// Confirmed on L1, with reference.
    Confirmed(CheckpointL1Ref),
    /// Finalized on L1, with reference
    Finalized(CheckpointL1Ref),
}
