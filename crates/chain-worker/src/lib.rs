//! # strata-chain-worker
//!
//! Chain worker implementation using the OL STF.
//!
//! This crate provides a dedicated asynchronous worker for managing Strata's
//! OL chainstate database. It encapsulates the logic for fetching, executing,
//! and finalizing OL blocks and epochs using:
//!
//! - OL STF ([`strata_ol_stf_v1::verify_block`])
//! - OL types ([`OLBlockV1`](strata_ol_chain_types_v1::OLBlockV1),
//!   [`OLBlockHeaderV1`](strata_ol_chain_types_v1::OLBlockHeaderV1),
//!   [`OLStateV1`](strata_ol_state_types_v1::OLStateV1),
//!   [`WriteBatch`](strata_ol_state_types_v1::WriteBatch))
//! - [`IndexerState<WriteTrackingState<OLStateV1>>`](strata_ol_state_support_types::IndexerState)
//!   for state tracking

mod context;
mod errors;
mod handle;
mod message;
mod mmr_prefill;
mod output;
mod service;
mod state;
mod traits;

#[cfg(test)]
mod tests;

pub use context::ChainWorkerContextImpl;
pub use errors::{WorkerError, WorkerResult};
pub use handle::ChainWorkerHandle;
pub use message::ChainWorkerMessage;
pub use mmr_prefill::{prefill_l1_block_refs_mmr, prefill_l1_block_refs_mmr_blocking};
pub use output::OLBlockExecutionOutput;
pub use service::{ChainWorkerService, ChainWorkerStatus, start_chain_worker_service_from_ctx};
pub use state::ChainWorkerServiceState;
pub use traits::ChainWorkerContext;
