//! Helpers for constructing and parsing SPS-50 checkpoint transactions.

pub mod constants;
pub mod errors;
pub mod parser;

pub use constants::{CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE};
pub use errors::{CheckpointTxError, CheckpointTxResult};
pub use parser::extract_signed_checkpoint_from_envelope;
// Re-export checkpoint types for consumers of parsed checkpoints.
pub use strata_checkpoint_types_new::{
    BatchInfo, BatchTransition, CheckpointClaim, CheckpointClaimBuilder, CheckpointCommitment,
    CheckpointPayload, CheckpointSidecar, EpochSummary, L1BlockRange, L2BlockRange,
    SignedCheckpointPayload, verify_checkpoint_payload_signature,
};
