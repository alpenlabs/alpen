//! SSZ types for checkpoint subprotocol.
//!
//! This crate provides SSZ-serializable types for:
//! - Checkpoint payloads posted to L1
//! - Checkpoint claims used for proof verification
//!
//! # Checkpoint Claim and Payload Relationship
//!
//! [`CheckpointClaim`] represents the complete public parameters for ZK proof verification.
//! It claims that in a checkpoint epoch:
//! - OL consumed L1 information from [`L1BlockRange::start`] to [`L1BlockRange::end`] (inclusive)
//! - OL executed blocks with [`L2BlockRange::start`] as the parent (last verified) and
//!   [`L2BlockRange::end`] as the final block
//! - All input messages consumed are represented by [`CheckpointClaim::input_msgs_commitment`]
//! - All output messages produced are in [`CheckpointSidecar`] (hashed as
//!   [`CheckpointClaim::ol_logs_hash`])
//! - The [`CheckpointClaim::state_diff_hash`] is the hash of the state diff in
//!   [`CheckpointSidecar`] between [`L2BlockRange::start`] and [`L2BlockRange::end`]
//!
//! However, [`CheckpointPayload`] (posted to L1) omits information already on L1:
//! - The last verified [`OLBlockCommitment`](strata_identifiers::OLBlockCommitment) (start) is not
//!   sent (already posted previously)
//! - L1 block commitments are not sent at all; only the L1 height range is needed since ASM
//!   verifies L1 blocks using its history accumulator
//!
//! This redundant information is already stored in the ASM's checkpoint state on L1.
//! ASM reconstructs the full [`CheckpointClaim`] from:
//! - The [`CheckpointPayload`] posted in the current transaction
//! - The checkpoint state stored in ASM (provides the last verified OL block commitment)
//! - The L1 history accumulator and auxiliary data (provides verified L1 block commitments from
//!   heights)
//!
//! This design minimizes L1 data costs by avoiding redundant information while maintaining
//! full verifiability through state reconstruction.

mod claim;
mod error;
mod payload;

pub use error::CheckpointPayloadError;
use strata_identifiers::{impl_borsh_via_ssz, impl_borsh_via_ssz_fixed};

/// SSZ-generated types for serialization and merkleization.
#[allow(
    clippy::all,
    clippy::absolute_paths,
    unreachable_pub,
    clippy::allow_attributes,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

// Re-export types from claim.ssz
pub use ssz_generated::ssz::claim::{
    CheckpointClaim, CheckpointClaimRef, CheckpointScope, CheckpointScopeRef, L1BlockHeightRange,
    L1BlockHeightRangeRef, L2BlockRange, L2BlockRangeRef,
};
// Re-export types from payload.ssz
pub use ssz_generated::ssz::payload::{
    CheckpointPayload, CheckpointPayloadRef, CheckpointSidecar, CheckpointSidecarRef,
    CheckpointTip, CheckpointTipRef, SignedCheckpointPayload, SignedCheckpointPayloadRef,
};
// Re-export constants from payload.ssz
pub use ssz_generated::ssz::payload::{MAX_PROOF_LEN, OL_DA_DIFF_MAX_SIZE, OUTPUT_MSG_MAX_SIZE};
// Re-export OLLog for consumers parsing checkpoint sidecar logs
pub use strata_ol_chain_types_new::OLLog;

// Borsh SSZ adapters for state persistence
// Fixed-size types (no length prefix needed)
impl_borsh_via_ssz_fixed!(L1BlockHeightRange);
impl_borsh_via_ssz_fixed!(L2BlockRange);
impl_borsh_via_ssz_fixed!(CheckpointScope);
impl_borsh_via_ssz_fixed!(CheckpointTip);

// Variable-size types (need length prefix for nesting)
impl_borsh_via_ssz!(CheckpointSidecar);
impl_borsh_via_ssz!(CheckpointPayload);
impl_borsh_via_ssz!(SignedCheckpointPayload);
impl_borsh_via_ssz!(CheckpointClaim);
