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
pub mod primitives;
pub mod registry;
mod registry_builder;
mod registry_handle;
mod registry_prover;
mod service;
mod state;
mod task;
mod task_id;
mod worker;
pub mod zkvm;

// Re-export core registry types at the top level
pub use registry::{
    BoxedInput, BoxedProof, ConcreteHandler, ProgramHandler, ProgramRegistry, ProgramType,
};
pub use registry::{InputFetcher as RegistryInputFetcher, ProofStore as RegistryProofStore};
pub use registry_builder::RegistryProverServiceBuilder;
pub use registry_handle::RegistryProverHandle;
pub use registry_prover::RegistryProver;
pub use task_id::TaskId;

// Re-export zkvm backend type
pub use zkvm::ZkVmBackend;

// Re-export legacy zkvm types for backward compatibility
pub use zkvm::{
    InputFetcher as ZkVmInputFetcher, ProgramId, ProofStore as ZkVmProofStore, ZkVmTaskId,
};

// Re-export framework types
pub use builder::ProverServiceBuilder;
pub use config::{PaaSConfig, RetryConfig, WorkerConfig};
pub use error::{PaaSError, PaaSResult};
pub use handle::ProverHandle;
pub use service::{ProverService, ProverServiceStatus};
pub use state::{ProverServiceState, StatusSummary};
pub use task::TaskStatus;

// Re-export primitives integration
pub use primitives::ProofContextVariant;

// Prover trait for custom implementations
//
// Users can implement this trait for custom proving strategies that need
// to dynamically resolve hosts or handle multiple backends.
pub trait Prover: Send + Sync + 'static {
    type TaskId: task::TaskId;
    type Backend: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static;

    fn backend(&self, task_id: &Self::TaskId) -> Self::Backend;

    fn prove(
        &self,
        task_id: Self::TaskId,
    ) -> impl std::future::Future<Output = PaaSResult<()>> + Send;
}
