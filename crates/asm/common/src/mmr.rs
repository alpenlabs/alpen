//! History accumulator for ASM.

use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as SszDecode, Encode as SszEncode};
use strata_asm_manifest_types::{AsmManifest, Hash32};
use strata_codec::{Codec, CodecError, Decoder, Encoder, decode_buf_exact, encode_to_vec};
use strata_merkle::{CompactMmr64, MerkleProof, Mmr, Sha256Hasher, error::MerkleError};

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
#[derive(
    Clone,
    Debug,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct AsmHistoryAccumulatorState {
    /// MMR accumulator for [`AsmManifest`]
    #[rkyv(with = CompactMmr64AsBytes)]
    manifest_mmr: CompactMmr64<Hash32>,
    /// Height offset for mapping MMR indices to L1 block heights.
    /// Equal to `genesis_height + 1` since manifests start after genesis.
    offset: u64,
}

/// Serializer for [`CompactMmr64<Hash32>`] as bytes for rkyv.
struct CompactMmr64AsBytes;

impl ArchiveWith<CompactMmr64<Hash32>> for CompactMmr64AsBytes {
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(
        field: &CompactMmr64<Hash32>,
        resolver: Self::Resolver,
        out: Place<Self::Archived>,
    ) {
        let bytes = encode_to_vec(field).expect("codec should serialize compact mmr");
        rkyv::Archive::resolve(&bytes, resolver, out);
    }
}

impl<S> SerializeWith<CompactMmr64<Hash32>, S> for CompactMmr64AsBytes
where
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(
        field: &CompactMmr64<Hash32>,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        let bytes = encode_to_vec(field).expect("codec should serialize compact mmr");
        rkyv::Serialize::serialize(&bytes, serializer)
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, CompactMmr64<Hash32>, D> for CompactMmr64AsBytes
where
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        deserializer: &mut D,
    ) -> Result<CompactMmr64<Hash32>, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(decode_buf_exact(&bytes).expect("codec should deserialize compact mmr"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SszEncode, SszDecode)]
struct AsmHistoryAccumulatorSsz {
    manifest_mmr: Vec<u8>,
    offset: u64,
}

impl Encode for AsmHistoryAccumulatorState {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        let bytes = encode_to_vec(&self.manifest_mmr).expect("codec should serialize compact mmr");
        AsmHistoryAccumulatorSsz {
            manifest_mmr: bytes,
            offset: self.offset,
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        let bytes = encode_to_vec(&self.manifest_mmr).expect("codec should serialize compact mmr");
        AsmHistoryAccumulatorSsz {
            manifest_mmr: bytes,
            offset: self.offset,
        }
        .ssz_bytes_len()
    }
}

impl Decode for AsmHistoryAccumulatorState {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let decoded = AsmHistoryAccumulatorSsz::from_ssz_bytes(bytes)?;
        let manifest_mmr = decode_buf_exact(&decoded.manifest_mmr).map_err(|err| {
            DecodeError::BytesInvalid(format!("invalid compact mmr bytes: {err}"))
        })?;
        Ok(Self {
            manifest_mmr,
            offset: decoded.offset,
        })
    }
}

impl AsmHistoryAccumulatorState {
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

impl Codec for AsmHistoryAccumulatorState {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.manifest_mmr.encode(enc)?;
        self.offset.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let manifest_mmr = CompactMmr64::<Hash32>::decode(dec)?;
        let offset = u64::decode(dec)?;
        Ok(Self {
            manifest_mmr,
            offset,
        })
    }
}
