//! Consensus validation logic and core state machine

// When debug-asm is enabled, StrataAsmSpec is not directly imported but
// strata-asm-spec is still needed as a transitive dependency (DebugAsmSpec wraps it).
#[cfg(feature = "debug-asm")]
use strata_asm_spec as _;

pub mod asm_worker_context;
mod asm_worker_submitter;
pub mod checkpoint_sync;
pub mod csm_worker_context;
mod fcm;
pub mod message;
pub mod ol_mmr_reconcile;
pub mod sync_manager;
#[cfg(test)]
mod test_utils;
pub mod tip_update;
pub mod unfinalized_tracker;

pub mod errors;
pub mod sync_handle;

pub use asm_worker_submitter::AsmBlockSubmitter;
pub use fcm::*;
pub use sync_handle::SyncServiceHandle;
