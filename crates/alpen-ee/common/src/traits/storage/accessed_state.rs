//! Per-block accessed-state and content-addressed bytecode storage.
//!
//! Producer: the `AccessedStateGenerator` exex (phase 2 of the EE prover
//! redesign), which writes one record per committed block plus any newly
//! seen bytecodes.
//!
//! Consumer: the chunk-builder at chunk-seal time, replacing today's
//! per-block re-execution loop inside `RangeWitnessExtractor`.

use async_trait::async_trait;
use strata_acct_types::Hash;

use crate::{AccessedStateRecord, StorageError};

/// Per-block accessed-state records + bytecode cache.
///
/// Two lifecycles share the trait because both feeds are written by the
/// same exex and read by the same consumer:
///
/// - **Per-block records** are tied to chain canonicality. On reorg, the
///   exex deletes records for the orphaned block hashes.
/// - **Bytecodes** are content-addressed by code hash. Once written they
///   are never deleted — same contract referenced by many chunks shares
///   one stored copy.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait AccessedStateStore: Send + Sync {
    /// Insert or overwrite the accessed-state record for `block_id`.
    async fn put_block_accessed_state(
        &self,
        block_id: Hash,
        record: AccessedStateRecord,
    ) -> Result<(), StorageError>;

    /// Fetch the accessed-state record for `block_id`, if persisted.
    async fn get_block_accessed_state(
        &self,
        block_id: Hash,
    ) -> Result<Option<AccessedStateRecord>, StorageError>;

    /// Remove the accessed-state record for `block_id`. Idempotent.
    async fn del_block_accessed_state(&self, block_id: Hash) -> Result<(), StorageError>;

    /// Insert a bytecode keyed by its code hash. Idempotent — bytecodes
    /// are content-addressed.
    async fn put_bytecode(&self, code_hash: Hash, code: Vec<u8>) -> Result<(), StorageError>;

    /// Fetch a bytecode by code hash, if present.
    async fn get_bytecode(&self, code_hash: Hash) -> Result<Option<Vec<u8>>, StorageError>;
}
