use std::sync::Arc;

use strata_db_types::{
    mmr_helpers::{MmrAlgorithm, MmrId},
    traits::UnifiedMmrDatabase,
    DbResult,
};
use strata_merkle::MerkleProofB32 as MerkleProof;
use threadpool::ThreadPool;

use crate::ops;

/// Unified manager for all MMR instances in the system
///
/// Provides a single entry point for managing multiple MMR instances
/// through the handle pattern. Each handle captures a specific MmrId
/// and provides ergonomic async/blocking APIs for that MMR.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct UnifiedMmrManager {
    ops: Arc<ops::unified_mmr::UnifiedMmrDataOps>,
}

impl UnifiedMmrManager {
    pub fn new(pool: ThreadPool, db: Arc<impl UnifiedMmrDatabase + 'static>) -> Self {
        let ops = Arc::new(ops::unified_mmr::Context::new(db).into_ops(pool));
        Self { ops }
    }

    /// Get a handle for a specific MMR instance
    ///
    /// The handle captures the MmrId and provides methods that don't require
    /// repeatedly passing the mmr_id parameter.
    pub fn get_handle(&self, mmr_id: MmrId) -> MmrHandle {
        MmrHandle {
            mmr_id,
            ops: self.ops.clone(),
        }
    }
}

/// Handle for a specific MMR instance
///
/// Provides ergonomic async/blocking APIs for a single MMR instance.
/// The MmrId is captured at creation time, so methods don't need to
/// repeatedly pass it.
pub struct MmrHandle {
    mmr_id: MmrId,
    ops: Arc<ops::unified_mmr::UnifiedMmrDataOps>,
}

impl MmrHandle {
    /// Append a new leaf to the MMR (async version)
    pub async fn append_leaf(&self, hash: [u8; 32]) -> DbResult<u64> {
        self.ops.append_leaf_async(self.mmr_id.clone(), hash).await
    }

    /// Append a new leaf to the MMR (blocking version)
    pub fn append_leaf_blocking(&self, hash: [u8; 32]) -> DbResult<u64> {
        self.ops
            .append_leaf_blocking(self.mmr_id.clone(), hash)
    }

    /// Get a node at a specific position (blocking)
    pub fn get_node_blocking(&self, pos: u64) -> DbResult<[u8; 32]> {
        self.ops.get_node_blocking(self.mmr_id.clone(), pos)
    }

    /// Get the total MMR size (blocking)
    pub fn mmr_size_blocking(&self) -> DbResult<u64> {
        self.ops.mmr_size_blocking(self.mmr_id.clone())
    }

    /// Get the number of leaves (blocking)
    pub fn num_leaves_blocking(&self) -> DbResult<u64> {
        self.ops.num_leaves_blocking(self.mmr_id.clone())
    }

    /// Generate a Merkle proof for a single leaf position
    pub fn generate_proof(&self, index: u64) -> DbResult<MerkleProof> {
        let mmr_size = self.mmr_size_blocking()?;
        let num_leaves = self.num_leaves_blocking()?;

        MmrAlgorithm::generate_proof(index, mmr_size, num_leaves, |pos| {
            self.get_node_blocking(pos)
        })
    }

    /// Generate Merkle proofs for a range of leaf positions
    pub fn generate_proofs(&self, start: u64, end: u64) -> DbResult<Vec<MerkleProof>> {
        let mmr_size = self.mmr_size_blocking()?;
        let num_leaves = self.num_leaves_blocking()?;

        MmrAlgorithm::generate_proofs(start, end, mmr_size, num_leaves, |pos| {
            self.get_node_blocking(pos)
        })
    }

    /// Remove and return the last leaf from the MMR (async version)
    pub async fn pop_leaf(&self) -> DbResult<Option<[u8; 32]>> {
        self.ops.pop_leaf_async(self.mmr_id.clone()).await
    }

    /// Remove and return the last leaf from the MMR (blocking version)
    pub fn pop_leaf_blocking(&self) -> DbResult<Option<[u8; 32]>> {
        self.ops.pop_leaf_blocking(self.mmr_id.clone())
    }

    /// Get the MmrId for this handle
    pub fn mmr_id(&self) -> &MmrId {
        &self.mmr_id
    }
}
