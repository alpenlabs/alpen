//! History accumulator for ASM.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
use strata_asm_manifest_types::{AsmManifest, Hash32};
use strata_merkle::{CompactMmr64, MerkleProof, Mmr, Mmr64B32, Sha256Hasher, error::MerkleError};
use tree_hash::{PackedEncoding, Sha256Hasher as TreeHashSha256Hasher, TreeHash, TreeHashType};
use tree_hash_derive::TreeHash;

/// Capacity of the ASM MMR as a power of 2.
///
/// With a value of 64, the MMR supports up to 2^64 leaves
const ASM_MMR_CAP_LOG2: u8 = 64;

/// The hasher used for ASM manifest MMR operations.
///
/// Uses SHA-256 with full 32-byte hash output.
pub type AsmHasher = Sha256Hasher;

pub type AsmMerkleProof = MerkleProof<Hash32>;

/// Verifiable accumulator for ASM's L1 block processing history.
///
/// Maintains a compact MMR of manifest hashes with a height offset for mapping MMR indices
/// to L1 block heights. The accumulator tracks the sequential processing of L1 blocks by the
/// ASM, starting from the first block after genesis.
///
/// # Index-to-Height Mapping
///
/// MMR index 0 corresponds to the manifest at L1 block height `genesis_height + 1`:
/// - `offset = genesis_height + 1`
/// - Height at MMR index `i` = `offset + i = genesis_height + 1 + i`
///
/// # Example
///
/// ```ignore
/// // Genesis is at L1 block 800000
/// let mut accumulator = AsmHistoryAccumulatorState::new(800000);
/// // Internal offset is 800001
///
/// accumulator.add_leaf(hash1)?; // MMR index 0 → L1 block height 800001
/// accumulator.add_leaf(hash2)?; // MMR index 1 → L1 block height 800002
/// accumulator.add_leaf(hash3)?; // MMR index 2 → L1 block height 800003
/// ```
#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AsmHistoryAccumulatorState {
    /// MMR accumulator for [`AsmManifest`]
    manifest_mmr: CompactMmr64<Hash32>,
    /// Height offset for mapping MMR indices to L1 block heights.
    /// Equal to `genesis_height + 1` since manifests start after genesis.
    offset: u64,
}

/// The SSZ representation of the [`AsmHistoryAccumulatorState`].
#[derive(DeriveEncode, DeriveDecode, TreeHash)]
struct AsmHistoryAccumulatorStateSsz {
    /// The serialized manifest MMR.
    manifest_mmr: Mmr64B32,
    /// The offset.
    offset: u64,
}

impl AsmHistoryAccumulatorState {
    /// Converts the [`AsmHistoryAccumulatorState`] to its SSZ representation.
    fn to_ssz(&self) -> AsmHistoryAccumulatorStateSsz {
        AsmHistoryAccumulatorStateSsz {
            manifest_mmr: Mmr64B32::from_generic(&self.manifest_mmr),
            offset: self.offset,
        }
    }

    /// Converts the SSZ representation of a [`AsmHistoryAccumulatorState`] to a
    /// [`AsmHistoryAccumulatorState`].
    fn from_ssz(value: AsmHistoryAccumulatorStateSsz) -> Self {
        Self {
            manifest_mmr: value.manifest_mmr.to_generic(),
            offset: value.offset,
        }
    }

    /// Creates a new compact MMR for the given genesis height.
    ///
    /// The internal `offset` is set to `genesis_height + 1` since manifests
    /// start from the first block after genesis.
    pub fn new(genesis_height: u64) -> Self {
        let manifest_mmr = CompactMmr64::new(ASM_MMR_CAP_LOG2);
        Self {
            manifest_mmr,
            offset: genesis_height + 1,
        }
    }

    /// Returns the height offset for MMR index-to-height conversion.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the current number of leaves in the manifest MMR.
    pub fn num_entries(&self) -> u64 {
        self.manifest_mmr.num_entries()
    }

    /// Returns the L1 block height of the last manifest inserted into the MMR.
    ///
    /// Returns the genesis height if the MMR is empty.
    pub fn last_inserted_height(&self) -> u64 {
        // offset + num_entries - 1 because num_entries() is the count but MMR indices start at 0
        self.offset + self.manifest_mmr.num_entries() - 1
    }

    /// Verifies a Merkle proof for a leaf in the MMR.
    pub fn verify_manifest_leaf(&self, proof: &AsmMerkleProof, leaf: &Hash32) -> bool {
        self.manifest_mmr.verify::<AsmHasher>(proof, leaf)
    }

    /// Adds a new leaf to the MMR.
    pub fn add_manifest_leaf(&mut self, leaf: Hash32) -> Result<(), MerkleError> {
        Mmr::<AsmHasher>::add_leaf(&mut self.manifest_mmr, leaf)
    }

    pub fn verify_manifest(&mut self, proof: &AsmMerkleProof, manifest: AsmManifest) -> bool {
        let leaf_hash = manifest.compute_hash();
        self.verify_manifest_leaf(proof, &leaf_hash)
    }

    pub fn add_manifest(&mut self, manifest: &AsmManifest) -> Result<(), MerkleError> {
        let leaf_hash = manifest.compute_hash();
        self.add_manifest_leaf(leaf_hash)
    }
}

impl Encode for AsmHistoryAccumulatorState {
    fn is_ssz_fixed_len() -> bool {
        <AsmHistoryAccumulatorStateSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <AsmHistoryAccumulatorStateSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.to_ssz().ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        self.to_ssz().ssz_bytes_len()
    }
}

impl Decode for AsmHistoryAccumulatorState {
    fn is_ssz_fixed_len() -> bool {
        <AsmHistoryAccumulatorStateSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <AsmHistoryAccumulatorStateSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self::from_ssz(
            AsmHistoryAccumulatorStateSsz::from_ssz_bytes(bytes)?,
        ))
    }
}

impl TreeHash for AsmHistoryAccumulatorState {
    fn tree_hash_type() -> TreeHashType {
        <AsmHistoryAccumulatorStateSsz as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> PackedEncoding {
        <AsmHistoryAccumulatorStateSsz as TreeHash>::tree_hash_packed_encoding(&self.to_ssz())
    }

    fn tree_hash_packing_factor() -> usize {
        <AsmHistoryAccumulatorStateSsz as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> <TreeHashSha256Hasher as tree_hash::TreeHashDigest>::Output {
        <AsmHistoryAccumulatorStateSsz as TreeHash>::tree_hash_root(&self.to_ssz())
    }
}
