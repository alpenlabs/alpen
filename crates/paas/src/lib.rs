//! General-purpose Prover-as-a-Service (PaaS) library
//!
//! This crate provides a flexible framework for managing proof generation tasks
//! with worker pools, retry logic, and lifecycle management. The actual proving
//! logic is provided by the caller through the `Prover` trait.

mod builder;
mod config;
mod error;
mod handle;
mod service;
mod state;
mod task;
mod worker;

pub use builder::ProverServiceBuilder;
pub use config::{PaaSConfig, RetryConfig, WorkerConfig};
pub use error::{PaaSError, PaaSResult};
pub use handle::ProverHandle;
pub use task::{TaskId, TaskStatus};

/// Trait that defines how to prove a task
///
/// The caller implements this trait to define the actual proving logic.
/// The PaaS framework handles task lifecycle, worker pooling, and retries.
pub trait Prover: Send + Sync + 'static {
    /// Task identifier type (must be unique, hashable, and serializable)
    type TaskId: TaskId;

    /// Backend identifier for worker pooling
    ///
    /// Tasks are grouped by backend, and each backend has its own worker pool.
    /// For example: "sp1", "native", "risc0", etc.
    type Backend: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static;

    /// Get the backend for a given task
    fn backend(&self, task_id: &Self::TaskId) -> Self::Backend;

    /// Prove a task
    ///
    /// This is called by a worker when the task is ready to be proven.
    /// The implementation should generate the proof and store it appropriately.
    ///
    /// Returns:
    /// - `Ok(())` if proof generation succeeded
    /// - `Err(PaaSError::TransientFailure(_))` if the error is transient and should be retried
    /// - `Err(PaaSError::PermanentFailure(_))` if the error is permanent and should not be retried
    fn prove(
        &self,
        task_id: Self::TaskId,
    ) -> impl std::future::Future<Output = PaaSResult<()>> + Send;
}
