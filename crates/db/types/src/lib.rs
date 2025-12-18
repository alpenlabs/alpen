//! Database for the Alpen codebase.

pub mod chainstate;
pub mod errors;
pub mod mmr_helpers;
mod ol_state;
pub mod traits;
pub mod types;

#[cfg(feature = "stubs")]
pub mod stubs;

/// Wrapper result type for database operations.
pub type DbResult<T> = anyhow::Result<T, errors::DbError>;

pub use errors::DbError;
pub use ol_state::{OLFinalizedState, OLStateDatabase, OLWriteBatch};
