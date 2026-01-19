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
//! - OL executed blocks with [`L2BlockRange::start`] as the parent (last verified) and
//!   [`L2BlockRange::end`] as the final block
//! - All ASM manifests (logs emitted per L1 block) consumed in order are represented by
//!   [`CheckpointClaim::asm_manifests_hash`]
//! - All output messages produced are in [`CheckpointSidecar`] (hashed as
//!   [`CheckpointClaim::ol_logs_hash`])
//! - The [`CheckpointClaim::state_diff_hash`] is the hash of the state diff in
//!   [`CheckpointSidecar`] between [`L2BlockRange::start`] and [`L2BlockRange::end`]
//!
//! [`CheckpointPayload`] posted to L1 omits redundant information:
//! - The last verified [`OLBlockCommitment`](strata_identifiers::OLBlockCommitment) (L2 start) is
//!   already stored in ASM's checkpoint state
//! - Includes L1 height to identify which L1 blocks were processed up to this checkpoint
//!
//! ASM reconstructs the full [`CheckpointClaim`] by combining:
//! - [`CheckpointPayload`] data (new tip, L1 height, state diff, logs)
//! - Last verified OL block commitment from ASM's checkpoint state
//! - ASM manifests fetched from auxiliary data using the L1 height range, then hashed to compute
//!   `asm_manifests_hash`
//!
//! This minimizes L1 data costs while maintaining full verifiability.

mod claim;
mod error;
mod payload;

pub use error::CheckpointPayloadError;
use ssz_types::FixedBytes;
use strata_crypto::hash;
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
    CheckpointClaim, CheckpointClaimRef, L2BlockRange, L2BlockRangeRef,
};
// Re-export types from payload.ssz
pub use ssz_generated::ssz::payload::{
    CheckpointPayload, CheckpointPayloadRef, CheckpointSidecar, CheckpointSidecarRef,
    CheckpointTip, CheckpointTipRef, SignedCheckpointPayload, SignedCheckpointPayloadRef,
};
// Re-export constants from payload.ssz
pub use ssz_generated::ssz::payload::{
    MAX_OL_LOGS_PER_CHECKPOINT, MAX_PROOF_LEN, OL_DA_DIFF_MAX_SIZE,
};
// Re-export OLLog for consumers parsing checkpoint sidecar logs
pub use strata_ol_chain_types_new::OLLog;

// Borsh SSZ adapters for state persistence
// Fixed-size types (no length prefix needed)
impl_borsh_via_ssz_fixed!(L2BlockRange);
impl_borsh_via_ssz_fixed!(CheckpointTip);

// Variable-size types (need length prefix for nesting)
impl_borsh_via_ssz!(CheckpointSidecar);
impl_borsh_via_ssz!(CheckpointPayload);
impl_borsh_via_ssz!(SignedCheckpointPayload);
impl_borsh_via_ssz!(CheckpointClaim);

/// Computes a hash commitment over all ASM manifests in an L1 block range.
///
/// Concatenates the manifest hashes for all L1 blocks in the range
/// and returns a single hash commitment over them.
pub fn compute_asm_manifests_hash(manifest_hashes: Vec<[u8; 32]>) -> FixedBytes<32> {
    let mut data = Vec::with_capacity(manifest_hashes.len() * 32);
    for h in manifest_hashes {
        data.extend_from_slice(h.as_ref());
    }
    let hash = hash::raw(&data);
    hash.into()
}
