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
mod graph;
mod linear_executor;
mod mem_store;
mod process;
mod schedule;
mod store;
#[cfg(test)]
mod test_support;

pub use artifact_cache::*;
pub use config::*;
pub use context::*;
pub use errors::*;
pub use graph::*;
pub use linear_executor::*;
pub use mem_store::*;
pub use process::*;
pub use schedule::*;
pub use store::*;
