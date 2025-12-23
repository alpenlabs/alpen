//! SSZ types for checkpoint subprotocol.
//!
//! This crate provides SSZ-serializable types for:
//! - Checkpoint payloads posted to L1
//! - Epoch summaries stored in subprotocol state
//! - Checkpoint claims used for proof verification

mod claim;
mod error;
mod payload;
mod state;

pub use error::CheckpointPayloadError;
use strata_identifiers::{impl_borsh_via_ssz, impl_borsh_via_ssz_fixed};

/// SSZ-generated types for serialization and merkleization.
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

// Re-export payload types
// Re-export claim types
// Re-export constants
// Re-export state types
pub use ssz_generated::ssz::{
    claim::{CheckpointClaim, CheckpointClaimRef},
    payload::{
        BatchInfo, BatchInfoRef, BatchTransition, BatchTransitionRef, CheckpointCommitment,
        CheckpointCommitmentRef, CheckpointPayload, CheckpointPayloadRef, CheckpointSidecar,
        CheckpointSidecarRef, L1BlockRange, L1BlockRangeRef, L1Commitment, L1CommitmentRef,
        L2BlockRange, L2BlockRangeRef, MAX_PROOF_LEN, OL_DA_DIFF_MAX_SIZE, OUTPUT_MSG_MAX_SIZE,
        SignedCheckpointPayload, SignedCheckpointPayloadRef,
    },
    state::{EpochSummary, EpochSummaryRef},
};
// Re-export OLLog for consumers parsing checkpoint sidecar logs
pub use strata_ol_chain_types_new::OLLog;

// Borsh bridges for state persistence
// Fixed-size types (no length prefix needed)
impl_borsh_via_ssz_fixed!(L1Commitment);
impl_borsh_via_ssz_fixed!(L1BlockRange);
impl_borsh_via_ssz_fixed!(L2BlockRange);
impl_borsh_via_ssz_fixed!(BatchInfo);
impl_borsh_via_ssz_fixed!(BatchTransition);
impl_borsh_via_ssz_fixed!(CheckpointCommitment);
impl_borsh_via_ssz_fixed!(EpochSummary);

// Variable-size types (need length prefix for nesting)
impl_borsh_via_ssz!(CheckpointSidecar);
impl_borsh_via_ssz!(CheckpointPayload);
impl_borsh_via_ssz!(SignedCheckpointPayload);
impl_borsh_via_ssz!(CheckpointClaim);
