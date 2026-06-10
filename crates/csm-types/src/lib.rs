//! Client State Machine (CSM) types.
mod client_state;
mod operation;
mod payload;
mod status;

// Re-export commonly used types for convenience
pub use client_state::{CheckpointL1Ref, CheckpointState, ClientState, L1Checkpoint};
pub use operation::ClientUpdateOutput;
// `L1Payload`/`PayloadIntent` are defined locally (see [`payload`]); the other
// payload types are re-exported from `strata-btc-types`.
pub use payload::{BlobSpec, L1Payload, L1PayloadError, PayloadDest, PayloadIntent, PayloadSpec};
pub use status::L1Status;
