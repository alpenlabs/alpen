//! Executors that drive a pipeline of processor stages across a chain graph.
//!
//! An executor is told which links to traverse by an external sync engine.  For
//! each one it runs the registered [`GChainProc`](strata_gchain_types::GChainProc)
//! stages, holds onto the artifacts they produce, and later commits or rolls
//! back whole paths of them.

#![expect(missing_debug_implementations, reason = "wrong!")]

mod artifact_cache;
mod config;
mod context;
mod dispatcher;
mod errors;
mod linear_executor;
mod process;
#[cfg(test)]
mod test_support;

pub use artifact_cache::*;
pub use context::*;
pub use dispatcher::*;
pub use errors::*;
pub use linear_executor::*;
pub use process::*;
