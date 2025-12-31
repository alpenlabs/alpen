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

#![allow(unused, reason = "in development")]

mod errors;
mod output;

use anyhow as _;
pub use errors::{WorkerError, WorkerResult};
pub use output::OLBlockExecutionOutput;
use serde as _;
// Placeholder uses for dependencies that will be used in subsequent modules.
// These will be removed as we implement each module.
use strata_checkpoint_types as _;
use strata_db_types as _;
use strata_ledger_types as _;
use strata_params as _;
use strata_service as _;
use strata_snark_acct_types as _;
use strata_status as _;
use tokio as _;
use tracing as _;
