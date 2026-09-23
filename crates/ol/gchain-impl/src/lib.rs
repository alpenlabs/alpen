//! Implementation of the gchain traits for the orchestration layer.
//!
//! The OL chain graph has two kinds of links: blocks, processed by executing
//! them, and checkpoints, processed by reconstructing the epoch's state from
//! the DA payload observed on L1.  The processor stages here handle both the
//! same way, so a sync driver can mix them along a single path.

#![expect(missing_debug_implementations, reason = "stores don't implement Debug")]

mod chain_provider;
mod chain_spec;
mod errors;
mod exec;
mod graph_types;
mod index;
mod providers;
mod step;

#[cfg(test)]
mod tests;

pub use chain_provider::{OLChainProvider, OLChainStore};
pub use chain_spec::OLChainSpec;
pub use errors::{InvalidLinkError, OLProcError};
pub use exec::{OLExecArtifact, OLExecOutput, OLExecProc};
pub use graph_types::*;
pub use index::{OLIndexArtifact, OLIndexProc};
pub use providers::{L1ManifestProvider, OLIndexStore, OLStateStore};
