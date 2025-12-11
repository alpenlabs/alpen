//! MMR Database types and trait definitions

use strata_merkle::{CompactMmr64B32 as CompactMmr64, MerkleProofB32 as MerkleProof};
use thiserror::Error;

// Hash type for 32-byte hashes
type Hash = [u8; 32];

/// Result type for MMR database operations
pub type MmrDbResult<T> = Result<T, MmrDbError>;

/// Errors that can occur during MMR database operations
#[derive(Debug, Error)]
pub enum MmrDbError {
    /// Requested leaf index does not exist
    #[error("Leaf not found at index {0}")]
    LeafNotFound(u64),

    /// Invalid index range specified
    #[error("Invalid index range: {start}..{end}")]
    InvalidRange { start: u64, end: u64 },

    /// Storage backend error
    #[error("Storage error: {0}")]
    Storage(String),

    /// MMR operation error
    #[error("MMR error: {0}")]
    Mmr(String),

    /// Proof generation failed
    #[error("Failed to generate proof for index {index}: {reason}")]
    ProofGenerationFailed { index: u64, reason: String },
}

/// MMR database trait for persistent proof generation
///
/// Implementations of this trait maintain MMR data in a way that allows
/// efficient proof generation for arbitrary leaf positions.
///
/// ## Design Invariants
///
/// - Leaves are indexed from 0 sequentially
/// - `append_leaf` is the only way to add data (append-only)
/// - `num_leaves()` always returns the total number of leaves added
/// - Proofs are valid against the current `root()`
pub trait MmrDatabase: Send + Sync {
    /// Append a new leaf to the MMR
    ///
    /// Returns the index of the newly added leaf.
    ///
    /// # Arguments
    ///
    /// * `hash` - The hash value to append as a new leaf
    ///
    /// # Returns
    ///
    /// The index (0-based) of the appended leaf.
    fn append_leaf(&mut self, hash: Hash) -> MmrDbResult<u64>;

    /// Generate a Merkle proof for a single leaf position
    ///
    /// # Arguments
    ///
    /// * `index` - The leaf index (0-based) to generate a proof for
    ///
    /// # Returns
    ///
    /// A `MerkleProof` that can be verified against `root()`.
    ///
    /// # Errors
    ///
    /// Returns `MmrDbError::LeafNotFound` if `index >= num_leaves()`.
    fn generate_proof(&self, index: u64) -> MmrDbResult<MerkleProof>;

    /// Generate Merkle proofs for a range of leaf positions (batch operation)
    ///
    /// This is more efficient than calling `generate_proof` multiple times
    /// for contiguous ranges.
    ///
    /// # Arguments
    ///
    /// * `start` - The starting leaf index (inclusive)
    /// * `end` - The ending leaf index (inclusive)
    ///
    /// # Returns
    ///
    /// A vector of `MerkleProof`s, one for each index in the range.
    ///
    /// # Errors
    ///
    /// Returns `MmrDbError::InvalidRange` if `start > end`.
    /// Returns `MmrDbError::LeafNotFound` if any index is out of bounds.
    fn generate_proofs(&self, start: u64, end: u64) -> MmrDbResult<Vec<MerkleProof>>;

    /// Get the total number of leaves in the MMR
    fn num_leaves(&self) -> u64;

    /// Get the individual peak roots
    ///
    /// Returns a slice of peak roots in the compact representation.
    /// Proofs are verified against the appropriate peak root based on proof height.
    fn peak_roots(&self) -> &[Hash];

    /// Get a compact representation of the MMR
    ///
    /// This is useful for serialization and verification without needing
    /// the full tree structure.
    fn to_compact(&self) -> CompactMmr64;
}
