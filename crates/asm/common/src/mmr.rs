//! MMR types for ASM manifests.

use strata_merkle::{MerkleMr64, MerkleProof, Sha256Hasher};

/// Capacity of the ASM MMR as a power of 2.
///
/// With a value of 64, the MMR supports up to 2^64 leaves
pub const ASM_MMR_CAP_LOG2: usize = 64;

/// The hasher used for ASM manifest MMR operations.
///
/// Uses SHA-256 with full 32-byte hash output.
pub type AsmHasher = Sha256Hasher;

// Re-export Hash32 from manifest-types to maintain backward compatibility
pub use strata_asm_manifest_types::Hash32;

/// Compact representation of the ASM manifest MMR using 64-bit indexing.
///
/// This compact form stores only the peak hashes and is used for efficient
/// serialization and storage in the chain view state.
pub type AsmCompactMmr = strata_merkle::CompactMmr64<Hash32>;

/// Full ASM manifest MMR with 64-bit indexing.
///
/// This is the working form of the MMR that supports append operations
/// and can be compacted for storage.
pub type AsmMmr = MerkleMr64<AsmHasher>;

pub type AsmMerkleProof = MerkleProof<Hash32>;
