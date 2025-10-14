//! Client State Machine (CSM) types.
mod client_state;
mod operation;
mod status;

// Re-export commonly used types for convenience
pub use client_state::{CheckpointL1Ref, CheckpointState, ClientState, L1Checkpoint};
pub use operation::{ClientUpdateOutput, SyncAction};
pub use status::L1Status;
// Re-export payload types from btc-types (they were moved there)
pub use strata_btc_types::payload::{BlobSpec, L1Payload, PayloadDest, PayloadIntent, PayloadSpec};
