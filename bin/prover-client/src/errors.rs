use strata_db_types::DbError;
use strata_primitives::proof::ProofKey;
use thiserror::Error;

/// Represents errors that can occur while performing proving tasks.
#[derive(Error, Debug)]
pub(crate) enum ProvingTaskError {
    /// Occurs when a requested proof is not found in the database.
    #[error("Proof with ID {0:?} does not exist in DB.")]
    ProofNotFound(ProofKey),

    /// Occurs when input to a task is deemed invalid.
    #[error("Invalid input: Expected {0:?}")]
    InvalidInput(String),

    /// Occurs when the required witness data for a proving task is missing.
    #[error("Witness not found")]
    WitnessNotFound,

    /// Represents a generic database error.
    #[error("Database error: {0:?}")]
    DatabaseError(DbError),

    /// Represents an error occurring during an RPC call.
    #[error("{0}")]
    RpcError(String),
}
