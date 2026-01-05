//! Concrete orchestration layer MMR types.
// TODO move this to its own crate, why is it in acct-types?

use strata_merkle::*;

/// The basic hasher we use for all the MMR stuff.
///
/// This is SHA-256 with the full 32 byte hash.
// TODO should this be blake3 and be only 20 bytes or something?
pub type StrataHasher = Sha256Hasher;

/// Compact 64 bit merkle mountain range.
pub type CompactMmr64 = Mmr64B32;

/// Compact 64 bit merkle mountain range reference.
pub type CompactMmr64Ref<'a> = Mmr64B32Ref<'a>;

/// 64 bit merkle mountain range.
pub type Mmr64 = Mmr64B32;

/// 64 bit merkle mountain range reference.
pub type Mmr64Ref<'a> = Mmr64B32Ref<'a>;

/// Universal MMR merkle proof.
pub type MerkleProof = MerkleProofB32;

/// Universal MMR merkle proof reference.
pub type MerkleProofRef<'a> = MerkleProofB32Ref<'a>;

/// Raw MMR merkle proof that doesn't have an embedded index.
pub type RawMerkleProof = RawMerkleProofB32;

/// Raw MMR merkle proof reference that doesn't have an embedded index.
pub type RawMerkleProofRef<'a> = RawMerkleProofB32Ref<'a>;
