//! MMR types for ASM manifests.

use strata_merkle::{MerkleMr64, Sha256Hasher};

/// Capacity of the ASM manifest MMR as a power of 2.
///
/// With a value of 64, the MMR supports up to 2^64 leaves, providing
/// effectively unlimited capacity for manifest history.
pub const ASM_MANIFEST_MMR_CAP_LOG2: usize = 64;

/// The hasher used for ASM manifest MMR operations.
///
/// Uses SHA-256 with full 32-byte hash output.
pub type AsmManifestHasher = Sha256Hasher;

/// Hash type for ASM manifest MMR nodes.
pub type AsmManifestHash = [u8; 32];

/// Compact representation of the ASM manifest MMR using 64-bit indexing.
///
/// This compact form stores only the peak hashes and is used for efficient
/// serialization and storage in the chain view state.
pub type AsmManifestCompactMmr = strata_merkle::CompactMmr64<AsmManifestHash>;

/// Full ASM manifest MMR with 64-bit indexing.
///
/// This is the working form of the MMR that supports append operations
/// and can be compacted for storage.
pub type AsmManifestMmr = MerkleMr64<AsmManifestHasher>;
