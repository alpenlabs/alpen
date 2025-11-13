//! Universal task identifier for registry-based PaaS

use serde::{Deserialize, Serialize};

use crate::registry::ProgramType;
use crate::ZkVmBackend;

/// Universal task identifier with program and backend
///
/// This is the new task ID type that works with the registry-based system.
/// Users pass their program type directly without needing to specify discriminants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(bound = "P: ProgramType")]
pub struct TaskId<P: ProgramType> {
    /// The program to prove
    pub program: P,
    /// Backend to use for proving
    pub backend: ZkVmBackend,
}

impl<P: ProgramType> TaskId<P> {
    /// Create a new task ID
    pub fn new(program: P, backend: ZkVmBackend) -> Self {
        Self { program, backend }
    }

    /// Get a reference to the program
    pub fn program(&self) -> &P {
        &self.program
    }

    /// Get a reference to the backend
    pub fn backend(&self) -> &ZkVmBackend {
        &self.backend
    }
}

// Note: This struct automatically implements task::TaskIdentifier via blanket impl.
// TaskId<P> is the public API for registry-based tasks.
