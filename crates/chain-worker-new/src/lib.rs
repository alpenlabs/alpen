//! # strata-chain-worker-new
//!
//! New chain worker implementation using the OL STF and new OL types.
//!
//! This crate provides a dedicated asynchronous worker for managing Strata's
//! OL chainstate database. It encapsulates the logic for fetching, executing,
//! and finalizing OL blocks and epochs using:
//!
//! - New OL STF (`strata-ol-stf::verify_block`)
//! - New OL types (`OLBlock`, `OLBlockHeader`, `OLState`, `WriteBatch`)
//! - `IndexerState<WriteTrackingState<OLState>>` for state tracking
//! - `GlobalMmrManager` for MMR operations

mod builder;
mod errors;
mod handle;
mod message;
mod output;
mod service;
mod traits;

pub use builder::ChainWorkerBuilder;
pub use errors::{WorkerError, WorkerResult};
pub use handle::{ChainWorkerHandle, WorkerShared};
pub use message::ChainWorkerMessage;
pub use output::OLBlockExecutionOutput;
pub use service::{ChainWorkerService, ChainWorkerServiceState, ChainWorkerStatus};
pub use traits::WorkerContext;
