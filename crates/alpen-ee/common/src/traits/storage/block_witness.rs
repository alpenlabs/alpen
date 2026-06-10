//! Per-block proof-witness storage.
//!
//! Producer: the EE block-production / import path, which computes each block's
//! depth-0 transition witness while the block is at tip and persists it here
//! before the block is accepted. Consumer: the chunk prover's input assembly,
//! which gathers the per-block witnesses for a chunk's blocks.
//!
//! The stored value is the codec-encoded `EvmPartialState` (kept as opaque
//! bytes here so this crate stays free of the EE-specific witness type); it is
//! the same encoding the chunk guest consumes via
//! `RawBlockData::raw_partial_pre_state`.

use async_trait::async_trait;
use strata_acct_types::Hash;

use crate::StorageError;

/// Per-block proof-witness store, keyed by execution block hash.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait BlockWitnessStore: Send + Sync {
    /// Insert or overwrite the witness for `block_id`.
    async fn put_block_witness(&self, block_id: Hash, witness: Vec<u8>) -> Result<(), StorageError>;

    /// Fetch the witness for `block_id`, if persisted.
    async fn get_block_witness(&self, block_id: Hash) -> Result<Option<Vec<u8>>, StorageError>;

    /// Remove the witness for `block_id`. Idempotent.
    async fn del_block_witness(&self, block_id: Hash) -> Result<(), StorageError>;
}
