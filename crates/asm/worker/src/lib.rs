//! # strata-asm-worker
//!
//! The `strata-asm-worker` crate provides a dedicated asynchronous worker
//! for managing Strata's Anchor state (ASM).

// When debug-asm is enabled, StrataAsmSpec is not directly used (DebugAsmSpec wraps it),
// but strata-asm-spec is still a dependency for its transitive types.
#[cfg(feature = "debug-asm")]
use strata_asm_spec as _;

mod aux_resolver;
mod builder;
mod constants;
mod errors;
mod handle;
mod message;
mod service;
mod state;
mod traits;

pub use aux_resolver::AuxDataResolver;
pub use builder::AsmWorkerBuilder;
pub use errors::{WorkerError, WorkerResult};
pub use handle::AsmWorkerHandle;
pub use message::SubprotocolMessage;
pub use service::{AsmWorkerService, AsmWorkerStatus};
pub use state::AsmWorkerServiceState;
pub use traits::WorkerContext;
