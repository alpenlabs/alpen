//! Consensus validation logic and core state machine

// When debug-asm is enabled, StrataAsmSpec is not directly imported but
// strata-asm-spec is still needed as a transitive dependency (DebugAsmSpec wraps it).
#[cfg(feature = "debug-asm")]
use strata_asm_spec as _;

pub mod asm_worker_context;
mod asm_worker_submitter;
pub mod chain_worker_context;
pub mod checkpoint_sync;
pub mod checkpoint_verification;
pub mod exec_worker_context;
mod fcm;
pub mod fork_choice_manager;
pub mod genesis;
pub mod message;
pub mod sync_manager;
pub mod tip_update;
pub mod unfinalized_tracker;

pub mod errors;

pub use asm_worker_submitter::AsmBlockSubmitter;
pub use fcm::*;
