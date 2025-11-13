//! Prover-as-a-Service (PaaS) library with zkaleido integration
//!
//! This crate provides a framework for managing zkaleido proof generation tasks
//! with worker pools, retry logic, and lifecycle management.
//!
//! ## Architecture
//!
//! PaaS is built around the registry pattern for dynamic program dispatch.
//! To use PaaS, you implement the registry traits:
//!
//! - `ProgramType`: Your program type with routing key
//! - `RegistryInputFetcher<P, Prog>`: Fetches inputs for zkaleido programs
//! - `RegistryProofStore<P>`: Stores completed proofs
//!
//! See the `registry` module documentation for complete examples.

use serde::{Deserialize, Serialize};

mod commands;
mod config;
mod error;
pub mod registry;
mod service;
mod state;
mod task;
mod task_id;
mod worker;

/// ZkVm backend identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZkVmBackend {
    /// Native execution (no proving)
    Native,
    /// SP1 prover
    SP1,
    /// RISC0 prover
    Risc0,
}

// Re-export core registry types at the top level
pub use registry::{
    BoxedInput, BoxedProof, ConcreteHandler, InputFetcher as RegistryInputFetcher,
    ProgramHandler, ProgramRegistry, ProgramType, ProofStore as RegistryProofStore,
    RegistryProverHandle, RegistryProverServiceBuilder, RegistryProver,
};
pub use task_id::TaskId;

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
