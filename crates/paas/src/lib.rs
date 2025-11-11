//! Prover-as-a-Service (PaaS) library with zkaleido integration
//!
//! This crate provides a framework for managing zkaleido proof generation tasks
//! with worker pools, retry logic, and lifecycle management.
//!
//! ## Architecture
//!
//! PaaS is built around zkaleido's `ZkVmProgram` and `ZkVmHost` abstractions.
//! To use PaaS, you implement two traits:
//!
//! - `InputFetcher<P>`: Fetches inputs for your zkaleido programs
//! - `ProofStore<P>`: Stores completed proofs
//!
//! Where `P` is your `ProgramId` type that identifies different zkaleido programs.
//!
//! ## Example
//!
//! See the documentation in the `zkvm` module for a complete example.

mod builder;
mod commands;
mod config;
mod error;
mod handle;
mod service;
mod state;
mod task;
mod worker;
pub mod zkvm;

// Re-export core zkvm types at the top level
pub use zkvm::{InputFetcher, ProgramId, ProofStore, ZkVmBackend, ZkVmTaskId};

// Re-export framework types
pub use builder::ProverServiceBuilder;
pub use config::{PaaSConfig, RetryConfig, WorkerConfig};
pub use error::{PaaSError, PaaSResult};
pub use handle::ProverHandle;
pub use state::StatusSummary;
pub use task::TaskStatus;

// Internal trait for the service framework (not part of public API)
pub(crate) trait Prover: Send + Sync + 'static {
    type TaskId: task::TaskId;
    type Backend: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static;

    fn backend(&self, task_id: &Self::TaskId) -> Self::Backend;

    fn prove(
        &self,
        task_id: Self::TaskId,
    ) -> impl std::future::Future<Output = PaaSResult<()>> + Send;
}
