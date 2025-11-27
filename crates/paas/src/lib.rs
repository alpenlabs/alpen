//! Prover-as-a-Service (PaaS) library with zkaleido integration
//!
//! This crate provides a framework for managing zkaleido proof generation tasks
//! with worker pools, retry logic, and lifecycle management.
//!
//! ## Architecture
//!
//! PaaS provides a flexible framework for proof generation. To use PaaS:
//!
//! 1. Implement `ProgramType` for your program enum with routing keys
//! 2. Implement `InputFetcher` to fetch proof inputs
//! 3. Implement `ProofStorer` to persist completed proofs
//! 4. Implement `HostResolver` to resolve zkVM hosts (single centralized method)
//! 5. Use `RemoteProofHandler` or implement `ProofHandler` directly
//!
//! The `HostResolver` trait provides a unified API for host resolution,
//! returning a `HostInstance` enum that wraps concrete host types. This design
//! centralizes all host resolution logic in the consumer code.
//!
//! See the handler and remote_handler modules for examples.

use serde::{Deserialize, Serialize};
// Re-export zkaleido traits for convenience
pub use zkaleido::{ZkVmRemoteHost, ZkVmRemoteProgram};

mod builder;
mod commands;
mod config;
mod error;
mod handle;
mod handler;
mod host;
mod persistence;
mod program;
mod remote_handler;
mod service;
mod state;
mod task;
mod timer;

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

// Re-export framework types
pub use builder::ProverServiceBuilder;
pub use config::{ProverServiceConfig, RetryConfig, WorkerConfig};
pub use error::{ProverServiceError, ProverServiceResult};
pub use handle::ProverHandle;
pub use handler::{BoxedInput, InputFetcher, ProofHandler, ProofStorer};
pub use host::{HostInstance, HostResolver};
pub use persistence::{TaskRecord, TaskStore};
pub use program::ProgramType;
pub use remote_handler::RemoteProofHandler;
pub use service::{ProverService, ProverServiceStatus};
pub use state::{ProverServiceState, StatusSummary};
pub use task::{TaskId, TaskResult, TaskStatus};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zkvm_backend_serialization() {
        // Test that ZkVmBackend can be serialized
        let backend = ZkVmBackend::Native;
        let json = serde_json::to_string(&backend).unwrap();
        assert!(json.contains("Native"));

        let backend = ZkVmBackend::SP1;
        let json = serde_json::to_string(&backend).unwrap();
        assert!(json.contains("SP1"));

        let backend = ZkVmBackend::Risc0;
        let json = serde_json::to_string(&backend).unwrap();
        assert!(json.contains("Risc0"));
    }
}
