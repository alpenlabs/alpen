//! Executors that drive a pipeline of processor stages across a chain graph.
//!
//! An executor is told which links to traverse by an external sync engine.  For
//! each one it runs the registered [`GChainProc`](strata_gchain_types::GChainProc)
//! stages, holds onto the artifacts they produce, and later commits or rolls
//! back whole paths of them.

#![expect(
    missing_debug_implementations,
    reason = "executors hold stage trait objects and stores that don't implement Debug"
)]

mod artifact_cache;
mod config;
mod context;
mod errors;
mod linear_executor;
mod mem_store;
mod process;
mod schedule;
mod stage_runner;
mod store;
#[cfg(test)]
mod test_support;
mod tracking;
mod traverse;

pub use artifact_cache::*;
pub use config::*;
pub use context::*;
pub use errors::*;
pub use linear_executor::*;
pub use mem_store::*;
pub use stage_runner::LinkOutcome;
pub use store::*;
pub use tracking::*;
