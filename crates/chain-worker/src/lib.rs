//! # strata-chain-worker
//!
//! The `strata-chain-worker` crate provides a dedicated asynchronous worker
//! for managing Strata’s chainstate database. It encapsulates the logic for
//! fetching, executing, and finalizing L2 blocks and epochs, while handling
//! errors, workload dispatch, and state access in a concurrent environment.

mod builder;
mod context;
mod errors;
mod handle;
mod message;
mod service;
mod state;
mod traits;

pub use builder::ChainWorkerBuilder;
pub use context::WorkerExecCtxImpl;
pub use errors::{WorkerError, WorkerResult};
pub use handle::{ChainWorkerHandle, ChainWorkerInput, WorkerShared};
pub use message::ChainWorkerMessage;
pub use service::{ChainWorkerService, ChainWorkerServiceState, ChainWorkerStatus};
pub use traits::WorkerContext;
