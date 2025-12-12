//! # strata-asm-worker
//!
//! The `strata-asm-worker` crate provides a dedicated asynchronous worker
//! for managing Strata's Anchor state (ASM).

mod aux_resolver;
mod builder;
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
