use strata_db::DbError;
use strata_primitives::proof::ProofKey;
use thiserror::Error;
use zkaleido::ZkVmError;

use crate::status::ProvingTaskStatus;

/// Represents errors that can occur while performing proving tasks.
///
/// This error type encapsulates various issues that may arise during
/// the lifecycle of a proving task, including serialization issues,
/// invalid state transitions, and database-related errors. Each variant
/// provides specific information about the encountered error, making
/// it easier to diagnose and handle failures.
#[derive(Error, Debug)]
pub(crate) enum ProvingTaskError {
    /// Occurs when the serialization of the EL block prover input fails.
    #[error("Failed to serialize the EL block prover input")]
    Serialization(#[from] bincode::Error),

    /// Occurs when Borsh deserialization of the input fails.
    #[error("Failed to borsh deserialize the input")]
    BorshSerialization(#[from] borsh::io::Error),

    /// Occurs when attempting to create a task with an ID that already exists.
    #[error("Task with ID {0:?} already exists.")]
    TaskAlreadyFound(ProofKey),

    /// Occurs when trying to access a task that does not exist.
    #[error("Task with ID {0:?} does not exist.")]
    TaskNotFound(ProofKey),

    /// Occurs when a required dependency for a task does not exist.
    #[error("Dependency with ID {0:?} does not exist.")]
    DependencyNotFound(ProofKey),

    /// Occurs when a requested proof is not found in the database.
    #[error("Proof with ID {0:?} does not exist in DB.")]
    ProofNotFound(ProofKey),

    /// Occurs when a state transition is invalid based on the task's current status.
    #[error("Invalid status transition: {0:?} -> {1:?}")]
    InvalidStatusTransition(ProvingTaskStatus, ProvingTaskStatus),

    /// Occurs when input to a task is deemed invalid.
    #[error("Invalid input: Expected {0:?}")]
    InvalidInput(String),

    /// Occurs when the required witness data for a proving task is missing.
    #[error("Witness not found")]
    WitnessNotFound,

    /// Occurs when a newly created proving task is expected but none is found.
    #[error("No tasks found after creation; at least one was expected")]
    NoTasksFound,

    /// Occurs when the witness data provided is invalid.
    #[error("{0}")]
    InvalidWitness(String),

    /// Represents a generic database error.
    #[error("Database error: {0:?}")]
    DatabaseError(DbError),

    /// Represents an error occurring during an RPC call.
    #[error("{0}")]
    RpcError(String),

    /// Represents an error returned by the ZKVM.
    #[error("{0:?}")]
    ZkVmError(ZkVmError),
}
