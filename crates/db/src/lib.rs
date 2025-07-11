//! Database for the Alpen codebase.

#![feature(btree_extract_if)] // remove when we remove the stubs

pub mod chainstate;
pub mod errors;
pub mod traits;
pub mod types;

#[cfg(feature = "stubs")]
pub mod stubs;

/// Wrapper result type for database operations.
pub type DbResult<T> = anyhow::Result<T, errors::DbError>;

pub use errors::DbError;
