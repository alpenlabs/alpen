//! Crate includes reusable utils for services that handle common behavior.
//! Such as initializing the tracing framework and whatever else.

use serde::{Deserialize, Serialize};

pub mod logging;
pub mod retry;

#[cfg(feature = "debug-utils")]
mod bail_manager;
#[cfg(feature = "debug-utils")]
pub use bail_manager::*;
#[cfg(feature = "debug-utils")]
mod worker_pause_manager;
#[cfg(feature = "debug-utils")]
pub use worker_pause_manager::*;
pub mod ws_client;

/// Checks to see if we should bail out.
// Stub for when we don't actually want to do anything.
#[cfg(not(feature = "debug-utils"))]
#[inline(always)]
pub const fn check_bail_trigger(_s: &str) {
    // nothing
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkerType {
    SyncWorker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    // Pause for seconds
    Pause(u64),
    // Pause until asked to resume
    PauseUntilResume,
    Resume,
}

#[cfg(not(feature = "debug-utils"))]
#[inline(always)]
pub async fn send_action_to_worker(_wtype: WorkerType, _action: Action) -> bool {
    // Noop
    true
}
