//! Concrete orchestration layer MMR types.

use strata_merkle::{MerkleMr64, Sha256Hasher};

/// The basic hasher we use for all the MMR stuff.
///
/// This is SHA-256 with the full 32 byte hash.
// TODO should this be blake3 and be only 20 bytes or something?
pub type StrataHasher = Sha256Hasher;

/// Universal orchestration layer type.
pub type Hash = [u8; 32];

/// Compact 64 bit merkle mountain range.
pub type CompactMmr64 = strata_merkle::CompactMmr64<StrataHasher>;

/// 64 bit merkle mountain range.
pub type Mmr64 = MerkleMr64<StrataHasher>;

/// Universal MMR merkle proof.
pub type MerkleProof = strata_merkle::MerkleProof<Hash>;

/// Raw MMR merkle proof that doesn't have an embedded index.
pub type RawMerkleProof = strata_merkle::RawMerkleProof<Hash>;
