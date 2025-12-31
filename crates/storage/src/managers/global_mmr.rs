use std::{marker::PhantomData, sync::Arc};

use borsh::{BorshDeserialize, BorshSerialize};
use strata_db_types::{
    mmr_helpers::{BitManipulatedMmrAlgorithm, MmrAlgorithm},
    traits::GlobalMmrDatabase,
    DbError, DbResult,
};
use strata_identifiers::{Hash, MmrId};
use strata_merkle::{MerkleHasher, MerkleProofB32 as MerkleProof, Sha256Hasher};
use threadpool::ThreadPool;

use crate::ops;

/// Global manager for all MMR instances in the system
///
/// Provides a single entry point for managing multiple MMR instances
/// through the handle pattern. Each handle captures a specific MmrId
/// and provides ergonomic async/blocking APIs for that MMR.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct GlobalMmrManager {
    ops: Arc<ops::global_mmr::GlobalMmrDataOps>,
    // TODO: add a cache
}

impl GlobalMmrManager {
    pub fn new(pool: ThreadPool, db: Arc<impl GlobalMmrDatabase + 'static>) -> Self {
        let ops = Arc::new(ops::global_mmr::Context::new(db).into_ops(pool));
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
    pub fn get_data_handle<T>(&self, mmr_id: MmrId) -> TypedMmrHandle<T, Sha256Hasher>
    where
        T: BorshSerialize + BorshDeserialize,
    {
        TypedMmrHandle {
            handle: self.get_handle(mmr_id),
            _phantom: PhantomData,
        }
    }

    /// Get a data handle with a custom hasher for a specific MMR instance
    ///
    /// Allows using a different hasher than the default Sha256Hasher.
    pub fn get_data_handle_with_hasher<T, H>(&self, mmr_id: MmrId) -> TypedMmrHandle<T, H>
    where
        T: BorshSerialize + BorshDeserialize,
        H: MerkleHasher<Hash = [u8; 32]>,
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
#[expect(
    missing_debug_implementations,
    reason = "Inner ops type doesn't have Debug implementation"
)]
pub struct MmrHandle {
    mmr_id: MmrId,
    ops: Arc<ops::global_mmr::GlobalMmrDataOps>,
}

impl MmrHandle {
    /// Append a new leaf to the MMR (async version)
    pub async fn append_leaf(&self, hash: Hash) -> DbResult<u64> {
        self.ops
            .append_leaf_async(self.mmr_id.to_bytes(), hash)
            .await
    }

    /// Append a new leaf to the MMR (blocking version)
    pub fn append_leaf_blocking(&self, hash: Hash) -> DbResult<u64> {
        self.ops.append_leaf_blocking(self.mmr_id.to_bytes(), hash)
    }

    /// Get a node at a specific position (blocking)
    pub fn get_node_blocking(&self, pos: u64) -> DbResult<Option<Hash>> {
        self.ops.get_node_blocking(self.mmr_id.to_bytes(), pos)
    }

    /// Get the total MMR size (blocking)
    pub fn get_mmr_size_blocking(&self) -> DbResult<u64> {
        self.ops.get_mmr_size_blocking(self.mmr_id.to_bytes())
    }

    /// Get the number of leaves (blocking)
    pub fn get_num_leaves_blocking(&self) -> DbResult<u64> {
        self.ops.get_num_leaves_blocking(self.mmr_id.to_bytes())
    }

    /// Generate a Merkle proof for a single leaf position
    pub fn generate_proof(&self, index: u64) -> DbResult<MerkleProof> {
        let mmr_size = self.get_mmr_size_blocking()?;

        BitManipulatedMmrAlgorithm::generate_proof(index, mmr_size, self.node_getter())
    }

    /// Generate Merkle proofs for a range of leaf positions
    pub fn generate_proofs(&self, start: u64, end: u64) -> DbResult<Vec<MerkleProof>> {
        let mmr_size = self.get_mmr_size_blocking()?;

        BitManipulatedMmrAlgorithm::generate_proofs(start, end, mmr_size, self.node_getter())
    }

    fn node_getter(&self) -> impl Fn(u64) -> DbResult<[u8; 32]> + '_ {
        |pos| {
            self.get_node_blocking(pos)?
                .map(|x| x.0)
                .ok_or(DbError::MmrLeafNotFound(pos))
        }
    }

    /// Remove and return the last leaf from the MMR (async version)
    pub async fn pop_leaf(&self) -> DbResult<Option<Hash>> {
        self.ops.pop_leaf_async(self.mmr_id.to_bytes()).await
    }

    /// Remove and return the last leaf from the MMR (blocking version)
    pub fn pop_leaf_blocking(&self) -> DbResult<Option<Hash>> {
        self.ops.pop_leaf_blocking(self.mmr_id.to_bytes())
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
///
/// Generic over the hasher type `H` to allow different hashing strategies.
#[expect(
    missing_debug_implementations,
    reason = "Inner MmrHandle doesn't have Debug implementation"
)]
pub struct TypedMmrHandle<T, H = Sha256Hasher> {
    handle: MmrHandle,
    _phantom: PhantomData<(T, H)>,
}

impl<T, H> TypedMmrHandle<T, H>
where
    T: BorshSerialize + BorshDeserialize,
    H: MerkleHasher<Hash = [u8; 32]>,
{
    /// Append data to the MMR (blocking version)
    ///
    /// Atomically stores both the MMR hash and the original data.
    pub fn append_blocking(&self, data: &T) -> DbResult<u64> {
        let bytes = borsh::to_vec(data).map_err(|e| DbError::CodecError(e.to_string()))?;
        let hash = H::hash_leaf(&bytes).into();

        self.handle.ops.append_leaf_with_preimage_blocking(
            self.handle.mmr_id.to_bytes(),
            hash,
            bytes,
        )
    }

    /// Append data to the MMR (async version)
    ///
    /// Atomically stores both the MMR hash and the original data.
    pub async fn append(&self, data: &T) -> DbResult<u64> {
        let bytes = borsh::to_vec(data).map_err(|e| DbError::CodecError(e.to_string()))?;
        let hash = H::hash_leaf(&bytes).into();

        self.handle
            .ops
            .append_leaf_with_preimage_async(self.handle.mmr_id.to_bytes(), hash, bytes)
            .await
    }

    /// Get data by leaf index (blocking version)
    ///
    /// Returns an error if no data exists at the given index or if deserialization fails.
    pub fn get_blocking(&self, index: u64) -> DbResult<T> {
        let bytes = self
            .handle
            .ops
            .get_preimage_blocking(self.handle.mmr_id.to_bytes(), index)?
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
            .get_preimage_async(self.handle.mmr_id.to_bytes(), index)
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
