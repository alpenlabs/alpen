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

mod commands;
mod config;
mod error;
pub mod registry;
mod service;
mod state;
mod task;
mod task_id;
mod worker;
pub mod zkvm;

// Re-export core registry types at the top level
pub use registry::{
    BoxedInput, BoxedProof, ConcreteHandler, InputFetcher as RegistryInputFetcher,
    ProgramHandler, ProgramRegistry, ProgramType, ProofStore as RegistryProofStore,
    RegistryProverHandle, RegistryProverServiceBuilder, RegistryProver,
};
pub use task_id::TaskId;

// Re-export zkvm backend type
pub use zkvm::ZkVmBackend;

// Re-export legacy zkvm types for backward compatibility
pub use zkvm::{
    InputFetcher as ZkVmInputFetcher, ProgramId, ProofStore as ZkVmProofStore, ZkVmTaskId,
};

// Re-export framework types
pub use config::{PaaSConfig, RetryConfig, WorkerConfig};
pub use error::{PaaSError, PaaSResult};
pub use service::{ProverService, ProverServiceStatus};
pub use state::{ProverServiceState, StatusSummary};
pub use task::TaskStatus;

// Prover trait for custom implementations
//
// Users can implement this trait for custom proving strategies that need
// to dynamically resolve hosts or handle multiple backends.
pub trait Prover: Send + Sync + 'static {
    type TaskId: task::TaskIdentifier;
    type Backend: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static;

    fn backend(&self, task_id: &Self::TaskId) -> Self::Backend;

    fn prove(
        &self,
        task_id: Self::TaskId,
    ) -> impl std::future::Future<Output = PaaSResult<()>> + Send;
}
