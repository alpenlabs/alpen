//! # strata-chain-worker-new
//!
//! New chain worker implementation using the OL STF and new OL types.
//!
//! This crate provides a dedicated asynchronous worker for managing Strata's
//! OL chainstate database. It encapsulates the logic for fetching, executing,
//! and finalizing OL blocks and epochs using:
//!
//! - New OL STF ([`strata_ol_stf::verify_block`])
//! - New OL types ([`OLBlock`](strata_ol_chain_types_new::OLBlock),
//!   [`OLBlockHeader`](strata_ol_chain_types_new::OLBlockHeader),
//!   [`OLState`](strata_ol_state_types::OLState),
//!   [`WriteBatch`](strata_ol_state_types::WriteBatch))
//! - [`IndexerState<WriteTrackingState<OLState>>`](strata_ol_state_support_types::IndexerState) for
//!   state tracking

mod builder;
mod errors;
mod handle;
mod message;
mod output;
mod service;
mod state;
mod traits;

pub use builder::ChainWorkerBuilder;
pub use errors::{WorkerError, WorkerResult};
pub use handle::ChainWorkerHandle;
pub use message::ChainWorkerMessage;
pub use output::OLBlockExecutionOutput;
pub use service::{ChainWorkerService, ChainWorkerStatus};
pub use state::ChainWorkerServiceState;
pub use traits::ChainWorkerContext;
