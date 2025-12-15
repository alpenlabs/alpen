use std::sync::Arc;

use strata_db_types::{
    mmr_helpers::{find_peak_for_pos, leaf_index_to_pos, parent_pos, sibling_pos},
    traits::MmrDatabase,
    DbError, DbResult,
};
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
    ///
    /// Proof generation is implemented at the manager level using database primitives.
    pub fn generate_proof(&self, index: u64) -> DbResult<MerkleProof> {
        // Check bounds
        let num_leaves = self.ops.num_leaves_blocking()?;
        if index >= num_leaves {
            return Err(DbError::MmrLeafNotFound(index));
        }

        let mmr_size = self.ops.mmr_size_blocking()?;

        // Convert leaf index to MMR position
        let leaf_pos = leaf_index_to_pos(index);

        // Find which peak this leaf belongs to
        let peak_pos = find_peak_for_pos(leaf_pos, mmr_size)?;

        // Collect sibling hashes along the path from leaf to peak
        let mut cohashes = Vec::new();
        let mut current_pos = leaf_pos;
        let mut current_height = 0u8;

        // Climb to peak, collecting siblings
        while current_pos < peak_pos {
            let sib_pos = sibling_pos(current_pos, current_height);
            let sibling_hash = self.ops.get_node_blocking(sib_pos)?;

            cohashes.push(sibling_hash);

            // Move to parent
            current_pos = parent_pos(current_pos, current_height);
            current_height += 1;
        }

        Ok(MerkleProof::from_cohashes(cohashes, index))
    }

    /// Generate Merkle proofs for a range of leaf positions
    pub fn generate_proofs(&self, start: u64, end: u64) -> DbResult<Vec<MerkleProof>> {
        // Validate range
        if start > end {
            return Err(DbError::MmrInvalidRange { start, end });
        }

        let num_leaves = self.ops.num_leaves_blocking()?;
        if end >= num_leaves {
            return Err(DbError::MmrLeafNotFound(end));
        }

        // Generate proof for each index in range
        let mut proofs = Vec::with_capacity((end - start + 1) as usize);

        for index in start..=end {
            let proof = self.generate_proof(index)?;
            proofs.push(proof);
        }

        Ok(proofs)
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
