//! Checkpoint Sync Service (CSS).
//!
//! Watches the CSM-published client state for newly finalized epochs and drives
//! the chain worker to reconstruct and persist their OL state from the L1-observed
//! checkpoints, then publishes the resulting OL sync status.

mod context;
mod errors;
mod input;
mod service;
mod state;

#[cfg(test)]
mod tests;

pub use context::CheckpointSyncCtx;
pub use service::{
    start_css_service, CheckpointSyncService, CheckpointSyncStatus, CssServiceHandle,
};
pub use state::CheckpointSyncState;
