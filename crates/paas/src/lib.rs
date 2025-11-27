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
//! See the handler module for examples.
//!
//! ## Module Organization
//!
//! - **service**: Core service runtime (service, state, commands, handle, builder)
//! - **scheduler**: Retry scheduler for delayed execution (internal)
//! - **handler**: Proof generation handlers (traits, remote handler, host resolution)
//! - Root-level modules: task, config, program, error, persistence (fundamental types)

use serde::{Deserialize, Serialize};
// Re-export zkaleido traits for convenience
pub use zkaleido::{ZkVmRemoteHost, ZkVmRemoteProgram};

// Core type modules at root
mod config;
mod error;
mod persistence;
mod program;
mod task;

// Domain modules
mod handler;
mod scheduler;
mod service;

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

// Re-export all public types
pub use config::{ProverServiceConfig, RetryConfig, WorkerConfig};
pub use error::{ProverServiceError, ProverServiceResult};
pub use handler::{
    BoxedInput, HostInstance, HostResolver, InputFetcher, ProofHandler, ProofStorer,
    RemoteProofHandler,
};
pub use persistence::{TaskRecord, TaskStore};
pub use program::ProgramType;
pub use service::{
    ProverHandle, ProverService, ProverServiceBuilder, ProverServiceState, ProverServiceStatus,
    StatusSummary,
};
pub use task::{TaskId, TaskResult, TaskStatus};

// Scheduler types are internal, not re-exported
// (used by service internals, not public API)

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
