//! Client State Machine (CSM) types for the Strata rollup.
//!
//! This crate contains types related to the client state machine, including:
//! - Payload types for L1 data availability
//! - Status types for L1 connectivity and state
//! - Client state types for checkpoint tracking
//! - Operation types for state transitions

mod client_state;
mod operation;
mod status;

// Re-export commonly used types for convenience
pub use client_state::{CheckpointL1Ref, CheckpointState, ClientState, L1Checkpoint};
pub use operation::{ClientUpdateOutput, SyncAction};
pub use status::L1Status;
// Re-export payload types from btc-types (they were moved there)
pub use strata_btc_types::payload::{BlobSpec, L1Payload, PayloadDest, PayloadIntent, PayloadSpec};
