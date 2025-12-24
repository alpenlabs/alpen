use std::sync::Arc;

use strata_db_types::{mmr_helpers::MmrAlgorithm, traits::MmrDatabase, DbResult};
use strata_merkle::MerkleProofB32 as MerkleProof;
use threadpool::ThreadPool;

use crate::ops;

/// Manager for MMR (Merkle Mountain Range) database operations
///
/// Provides high-level async/blocking APIs for MMR operations including
/// appending leaves, generating proofs, and accessing MMR metadata.
///
/// Proof generation is implemented at the manager level using lower-level
/// database operations, keeping the database abstraction clean and simple.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct MmrManager {
    ops: ops::mmr::MmrDataOps,
}

impl MmrManager {
    pub fn new(pool: ThreadPool, db: Arc<impl MmrDatabase + 'static>) -> Self {
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

    /// Generate a Merkle proof for a single leaf position
    pub fn generate_proof(&self, index: u64) -> DbResult<MerkleProof> {
        let mmr_size = self.ops.mmr_size_blocking()?;
        let num_leaves = self.ops.num_leaves_blocking()?;

        MmrAlgorithm::generate_proof(index, mmr_size, num_leaves, |pos| {
            self.ops.get_node_blocking(pos)
        })
    }

    /// Generate Merkle proofs for a range of leaf positions
    pub fn generate_proofs(&self, start: u64, end: u64) -> DbResult<Vec<MerkleProof>> {
        let mmr_size = self.ops.mmr_size_blocking()?;
        let num_leaves = self.ops.num_leaves_blocking()?;

        MmrAlgorithm::generate_proofs(start, end, mmr_size, num_leaves, |pos| {
            self.ops.get_node_blocking(pos)
        })
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
