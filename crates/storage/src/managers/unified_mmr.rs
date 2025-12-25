use std::{marker::PhantomData, sync::Arc};

use borsh::{BorshDeserialize, BorshSerialize};
use strata_db_types::{
    mmr_helpers::{MmrAlgorithm, MmrId},
    traits::UnifiedMmrDatabase,
    DbError, DbResult,
};
use strata_merkle::{hasher::MerkleHasher, MerkleProofB32 as MerkleProof, Sha256Hasher};
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

    /// Get a handle for a specific MMR instance (hash-only operations)
    ///
    /// The handle captures the MmrId and provides methods that don't require
    /// repeatedly passing the mmr_id parameter.
    pub fn get_handle(&self, mmr_id: MmrId) -> MmrHandle {
        MmrHandle {
            mmr_id,
            ops: self.ops.clone(),
        }
    }

    /// Get a data handle for a specific MMR instance (with pre-image storage)
    ///
    /// The data handle stores and retrieves pre-image data alongside MMR hashes,
    /// providing type-safe access to the original data.
    pub fn get_data_handle<T>(&self, mmr_id: MmrId) -> TypedMmrHandle<T>
    where
        T: BorshSerialize + BorshDeserialize,
    {
        TypedMmrHandle {
            handle: self.get_handle(mmr_id),
            _phantom: PhantomData,
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

/// Typed handle for an MMR instance with pre-image storage
///
/// Provides type-safe operations for MMRs that store the original data
/// alongside the hash-based MMR structure. Wraps a hash-only MmrHandle
/// and adds typed data operations on top.
pub struct TypedMmrHandle<T> {
    handle: MmrHandle,
    _phantom: PhantomData<T>,
}

impl<T> TypedMmrHandle<T>
where
    T: BorshSerialize + BorshDeserialize,
{
    /// Append data to the MMR (blocking version)
    ///
    /// Atomically stores both the MMR hash and the original data.
    pub fn append_blocking(&self, data: &T) -> DbResult<u64> {
        let bytes = borsh::to_vec(data).map_err(|e| DbError::CodecError(e.to_string()))?;
        let hash = Sha256Hasher::hash_leaf(&bytes);

        self.handle
            .ops
            .append_leaf_with_preimage_blocking(self.handle.mmr_id.clone(), hash, bytes)
    }

    /// Append data to the MMR (async version)
    ///
    /// Atomically stores both the MMR hash and the original data.
    pub async fn append(&self, data: &T) -> DbResult<u64> {
        let bytes = borsh::to_vec(data).map_err(|e| DbError::CodecError(e.to_string()))?;
        let hash = Sha256Hasher::hash_leaf(&bytes);

        self.handle
            .ops
            .append_leaf_with_preimage_async(self.handle.mmr_id.clone(), hash, bytes)
            .await
    }

    /// Get data by leaf index (blocking version)
    ///
    /// Returns an error if no data exists at the given index or if deserialization fails.
    pub fn get_blocking(&self, index: u64) -> DbResult<T> {
        let bytes = self
            .handle
            .ops
            .get_preimage_blocking(self.handle.mmr_id.clone(), index)?
            .ok_or_else(|| {
                DbError::Other(format!(
                    "No pre-image data found for MMR {:?} at index {}",
                    self.handle.mmr_id, index
                ))
            })?;

        borsh::from_slice(&bytes).map_err(|e| DbError::CodecError(e.to_string()))
    }

    /// Get data by leaf index (async version)
    ///
    /// Returns an error if no data exists at the given index or if deserialization fails.
    pub async fn get(&self, index: u64) -> DbResult<T> {
        let bytes = self
            .handle
            .ops
            .get_preimage_async(self.handle.mmr_id.clone(), index)
            .await?
            .ok_or_else(|| {
                DbError::Other(format!(
                    "No pre-image data found for MMR {:?} at index {}",
                    self.handle.mmr_id, index
                ))
            })?;

        borsh::from_slice(&bytes).map_err(|e| DbError::CodecError(e.to_string()))
    }

    /// Get the underlying hash-only handle
    pub fn as_handle(&self) -> &MmrHandle {
        &self.handle
    }

    /// Get the MmrId for this handle
    pub fn mmr_id(&self) -> &MmrId {
        &self.handle.mmr_id
    }
}
