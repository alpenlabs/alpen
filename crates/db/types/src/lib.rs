//! Database for the Alpen codebase.

pub mod asm;
pub mod backend;
pub mod checkpoint_proof;
pub mod chunked_envelope;
pub mod client_state;
pub mod common;
pub mod errors;
pub mod l1;
pub mod l1_broadcast;
pub mod l1_writer;
pub mod legacy;
pub mod mempool;
pub mod mmr_index;
pub mod ol_block;
pub mod ol_checkpoint;
pub mod ol_state;
pub mod ol_state_index;
pub mod prover_task;

/// Wrapper result type for database operations.
pub type DbResult<T> = anyhow::Result<T, errors::DbError>;

pub use errors::DbError;
pub use mmr_index::{
    num_leaves_to_mmr_size, BatchWrite, LeafPos, MmrBatchWrite, MmrId, MmrIndexPrecondition,
    MmrNodePos, MmrNodeTable, NodePos, NodeTable, RawMmrId,
};
pub use ol_state_index::*;
