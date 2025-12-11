//! Checkpoint types for the ASM checkpoint subprotocol.
//!
//! This crate provides SSZ-serializable checkpoint types used in the checkpoint
//! subprotocol for L1 posting, verification, and state management.
//!
//! # Serialization
//!
//! All types use SSZ (Simple Serialize) as the primary serialization format:
//! - L1 checkpoint transactions are SSZ-encoded
//! - Types implement `ssz::Encode` and `ssz::Decode` for wire format
//! - Types implement `tree_hash::TreeHash` for merkleization
//! - Borsh compatibility is provided via `impl_borsh_via_ssz!` macros for internal state
//!   serialization (subprotocol state DB)
//!
//! # Type Naming Convention
//!
//! - [`CheckpointPayload`]: Data posted to L1 (the on-chain artifact)
//! - [`SignedCheckpointPayload`]: Signed payload for L1 posting
//! - [`CheckpointClaim`]: Public input for proof verification
//! - [`EpochSummary`]: Summary of a verified checkpoint epoch (for state)

mod claim;
mod epoch;
mod error;
mod payload;
mod signature;

pub use claim::{CheckpointClaim, CheckpointClaimBuilder};
pub use epoch::EpochSummary;
pub use error::{CheckpointError, CheckpointResult};
pub use payload::{
    BatchInfo, BatchTransition, CheckpointCommitment, CheckpointPayload, CheckpointSidecar,
    L1BlockRange, L2BlockRange, SignedCheckpointPayload,
};
pub use signature::verify_checkpoint_payload_signature;

// Legacy type alias for backward compatibility during migration.
// TODO: Remove once all code is migrated to use `SignedCheckpointPayload`.
pub type SignedCheckpoint = SignedCheckpointPayload;
// Re-export commonly used identifier types
pub use strata_identifiers::{
    Buf32, Buf64, CredRule, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, L1Height,
    L2BlockCommitment, OLBlockId, Slot,
};
