use std::sync::Arc;

use strata_db_types::{traits::MmrDatabase, DbResult};
use strata_merkle::MerkleProofB32 as MerkleProof;
use threadpool::ThreadPool;

use crate::ops;

/// Manager for MMR (Merkle Mountain Range) database operations
///
/// Provides high-level async/blocking APIs for MMR operations including
/// appending leaves, generating proofs, and accessing MMR metadata.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct MmrManager {
    ops: ops::mmr::MmrDataOps,
}

impl MmrManager {
    pub fn new<D: MmrDatabase + 'static>(pool: ThreadPool, db: Arc<D>) -> Self {
        let ops = ops::mmr::Context::new(db).into_ops(pool);
        Self { ops }
    }

    /// Append a new leaf to the MMR (async version)
    ///
    /// Returns the index of the newly appended leaf.
    pub async fn append_leaf(&self, hash: [u8; 32]) -> DbResult<u64> {
        self.ops.append_leaf_async(hash).await
    }

    /// Append a new leaf to the MMR (blocking version)
    ///
    /// Returns the index of the newly appended leaf.
    pub fn append_leaf_blocking(&self, hash: [u8; 32]) -> DbResult<u64> {
        self.ops.append_leaf_blocking(hash)
    }

    /// Generate a Merkle proof for a single leaf position (async)
    pub async fn generate_proof(&self, index: u64) -> DbResult<MerkleProof> {
        self.ops.generate_proof_async(index).await
    }

    /// Generate a Merkle proof for a single leaf position (blocking)
    pub fn generate_proof_blocking(&self, index: u64) -> DbResult<MerkleProof> {
        self.ops.generate_proof_blocking(index)
    }

    /// Generate Merkle proofs for a range of leaf positions (async)
    pub async fn generate_proofs(&self, start: u64, end: u64) -> DbResult<Vec<MerkleProof>> {
        self.ops.generate_proofs_async(start, end).await
    }

    /// Generate Merkle proofs for a range of leaf positions (blocking)
    pub fn generate_proofs_blocking(&self, start: u64, end: u64) -> DbResult<Vec<MerkleProof>> {
        self.ops.generate_proofs_blocking(start, end)
    }

    /// Remove and return the last leaf from the MMR (async version)
    ///
    /// Returns `Some(hash)` if a leaf was removed, or `None` if the MMR is empty.
    pub async fn pop_leaf(&self) -> DbResult<Option<[u8; 32]>> {
        self.ops.pop_leaf_async().await
    }

    /// Remove and return the last leaf from the MMR (blocking version)
    ///
    /// Returns `Some(hash)` if a leaf was removed, or `None` if the MMR is empty.
    pub fn pop_leaf_blocking(&self) -> DbResult<Option<[u8; 32]>> {
        self.ops.pop_leaf_blocking()
    }
}
