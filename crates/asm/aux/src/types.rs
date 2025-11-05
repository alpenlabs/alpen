//! Common types used throughout the auxiliary framework.

use strata_asm_common::AsmManifestHash;

/// Index of a transaction within an L1 block (0-based).
///
/// Used to key auxiliary requests and responses to specific transactions
/// within a block during pre-processing and processing phases.
pub type L1TxIndex = usize;

/// Merkle proof for the manifest MMR.
///
/// Proves that a specific `AsmManifestHash` is committed in the MMR
/// at a particular position.
pub type ManifestMmrProof = strata_merkle::MerkleProof<AsmManifestHash>;
