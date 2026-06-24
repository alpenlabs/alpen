//! Per-block proof-witness storage.
//!
//! Producer: the EE block-production path, which harvests each block's raw
//! depth-0 witness parts (trie-node bag, bytecodes, BLOCKHASH ancestor headers)
//! while the block is at tip and persists them here before the block is
//! accepted. Consumer: the chunk prover's input assembly, which unions the
//! per-block node bags for a chunk's blocks into one chunk-level pre-state.
//!
//! The stored value is the encoded per-block witness record (kept as opaque
//! bytes here so this crate stays free of the EE-specific witness type).

use async_trait::async_trait;
use strata_acct_types::Hash;

use crate::StorageError;

/// Per-block proof-witness store, keyed by execution block hash.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait BlockWitnessStore: Send + Sync {
    /// Insert or overwrite the witness for `block_id`.
    async fn put_block_witness(&self, block_id: Hash, witness: Vec<u8>)
        -> Result<(), StorageError>;

    /// Fetch the witness for `block_id`, if persisted.
    async fn get_block_witness(&self, block_id: Hash) -> Result<Option<Vec<u8>>, StorageError>;

    /// Remove the witness for `block_id`. Idempotent.
    async fn del_block_witness(&self, block_id: Hash) -> Result<(), StorageError>;
}
