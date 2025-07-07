//! # strata-chain-worker
//!
//! The `strata-chain-worker` crate provides a dedicated asynchronous worker
//! for managing Strata’s chainstate database. It encapsulates the logic for
//! fetching, executing, and finalizing L2 blocks and epochs, while handling
//! errors, workload dispatch, and state access in a concurrent environment.

mod errors;
mod handle;
mod message;
mod state;
mod traits;
mod worker;

pub use errors::{WorkerError, WorkerResult};
pub use handle::{ChainWorkerHandle, ChainWorkerInput, WorkerShared};
pub use message::ChainWorkerMessage;
pub use traits::WorkerContext;
pub use worker::{init_worker_state, worker_task};
