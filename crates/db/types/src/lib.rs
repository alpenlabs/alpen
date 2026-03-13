//! Database for the Alpen codebase.

pub mod chainstate;
pub mod errors;
mod mmr_index;
pub mod traits;
pub mod types;

#[cfg(feature = "stubs")]
pub mod stubs;

/// Wrapper result type for database operations.
pub type DbResult<T> = anyhow::Result<T, errors::DbError>;

pub use errors::DbError;
pub use mmr_index::{
    num_leaves_to_mmr_size, BatchWrite, LeafPos, MmrBatchWrite, MmrId, MmrIndexPrecondition,
    MmrNodePos, MmrNodeTable, NodePos, NodeTable, RawMmrId,
};
