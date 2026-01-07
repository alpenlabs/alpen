use std::{fmt, str};

#[cfg(feature = "bitcoin")]
use arbitrary::Error as ArbitraryError;
use arbitrary::{Arbitrary, Result as ArbitraryResult, Unstructured};
// Re-export bitcoin types for internal use
#[cfg(feature = "bitcoin")]
pub(crate) use bitcoin::{BlockHash, absolute};
use borsh::{BorshDeserialize, BorshSerialize};
use const_hex as hex;
use hex::encode_to_slice;
use serde::{Deserialize, Serialize};
#[cfg(feature = "bitcoin")]
use serde::{Deserializer, Serializer, de, ser};
use ssz_derive::{Decode, Encode};
#[cfg(feature = "bitcoin")]
use ssz_types::view::ToOwnedSsz;
use strata_codec::{Codec, CodecError, Decoder, Encoder};

// Use generated type when bitcoin feature is not enabled
#[cfg(not(feature = "bitcoin"))]
use crate::ssz_generated::ssz::commitments::L1BlockCommitment;
// Import SSZ-generated ref type for ToOwnedSsz impl
#[cfg(feature = "bitcoin")]
use crate::ssz_generated::ssz::commitments::L1BlockCommitmentRef;
use crate::{buf::Buf32, hash::sha256d};

/// The bitcoin block height
pub type BitcoinBlockHeight = u64;

/// L1 block height (as a simple u32)
pub type L1Height = u32;

/// ID of an L1 block, usually the hash of its header.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Deserialize,
    Serialize,
    Encode,
    Decode,
)]
pub struct L1BlockId(Buf32);

impl L1BlockId {
    /// Computes the [`L1BlockId`] from the header buf. This is expensive in proofs and
    /// should only be done when necessary.
    pub fn compute_from_header_buf(buf: &[u8]) -> L1BlockId {
        Self::from(sha256d(buf))
    }
}

// Custom implementation without Debug/Display to avoid conflicts
impl From<Buf32> for L1BlockId {
    fn from(value: Buf32) -> Self {
        Self(value)
    }
}

impl From<L1BlockId> for Buf32 {
    fn from(value: L1BlockId) -> Self {
        value.0
    }
}

impl Codec for L1BlockId {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.0.encode(enc)
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let buf = Buf32::decode(dec)?;
        Ok(Self(buf))
    }
}

impl AsRef<[u8; 32]> for L1BlockId {
    fn as_ref(&self) -> &[u8; 32] {
        self.0.as_ref()
    }
}

// Manual TreeHash implementation for transparent wrapper
crate::impl_ssz_transparent_buf32_wrapper!(L1BlockId);

#[cfg(feature = "bitcoin")]
impl From<BlockHash> for L1BlockId {
    fn from(value: BlockHash) -> Self {
        L1BlockId(value.into())
    }
}

#[cfg(feature = "bitcoin")]
impl From<L1BlockId> for BlockHash {
    fn from(value: L1BlockId) -> Self {
        use bitcoin::hashes::Hash;
        BlockHash::from_byte_array(value.0.into())
    }
}

/// Witness transaction ID merkle root from a Bitcoin block.
///
/// This is the merkle root of all witness transaction IDs (wtxids) in a block.
/// Used instead of the regular transaction merkle root to include witness data
/// for complete transaction verification and malleability protection.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Deserialize,
    Serialize,
    Encode,
    Decode,
)]
pub struct WtxidsRoot(Buf32);

// Implement standard wrapper traits (Debug, Display, From, AsRef, Codec)
crate::impl_buf_wrapper!(WtxidsRoot, Buf32, 32);

// Manual TreeHash implementation for transparent wrapper
crate::impl_ssz_transparent_buf32_wrapper!(WtxidsRoot);

/// L1 Block commitment with block height and ID.
///
/// When bitcoin feature is enabled, uses absolute::Height internally.
/// When disabled, the generated SSZ type (with u32) is used instead.
#[cfg(feature = "bitcoin")]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct L1BlockCommitment {
    height: absolute::Height,
    blkid: L1BlockId,
}

// Manual SSZ implementation for bitcoin feature (since absolute::Height doesn't impl Encode/Decode)
#[cfg(feature = "bitcoin")]
impl ssz::Encode for L1BlockCommitment {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        4 + 32 // u32 + Buf32
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        let height_u32 = self.height.to_consensus_u32();
        height_u32.ssz_append(buf);
        self.blkid.ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        Self::ssz_fixed_len()
    }
}

#[cfg(feature = "bitcoin")]
impl ssz::Decode for L1BlockCommitment {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        4 + 32 // u32 + Buf32
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, ssz::DecodeError> {
        if bytes.len() != Self::ssz_fixed_len() {
            return Err(ssz::DecodeError::InvalidByteLength {
                len: bytes.len(),
                expected: Self::ssz_fixed_len(),
            });
        }

        let height_u32 = u32::from_ssz_bytes(&bytes[0..4])?;
        let height = absolute::Height::from_consensus(height_u32).map_err(|_| {
            ssz::DecodeError::BytesInvalid(format!("Invalid height: {}", height_u32))
        })?;
        let blkid = L1BlockId::from_ssz_bytes(&bytes[4..36])?;

        Ok(Self { height, blkid })
    }
}

// Manual TreeHash implementation for bitcoin feature
#[cfg(feature = "bitcoin")]
impl<H: tree_hash::TreeHashDigest> tree_hash::TreeHash<H> for L1BlockCommitment {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        tree_hash::TreeHashType::Container
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        unreachable!("Struct should never be packed.")
    }

    fn tree_hash_packing_factor() -> usize {
        unreachable!("Struct should never be packed.")
    }

    fn tree_hash_root(&self) -> H::Output {
        use tree_hash::TreeHash;
        let height_u32 = self.height.to_consensus_u32();
        let mut hasher = tree_hash::MerkleHasher::<H>::with_leaves(2);
        hasher
            .write(TreeHash::<H>::tree_hash_root(&height_u32).as_ref())
            .expect("should not apply too many leaves");
        hasher
            .write(TreeHash::<H>::tree_hash_root(&self.blkid).as_ref())
            .expect("should not apply too many leaves");
        hasher.finish().expect("should not have a remaining buffer")
    }
}

// Use macro to generate Borsh implementations via SSZ (fixed-size, no length prefix)
crate::impl_borsh_via_ssz_fixed!(L1BlockCommitment);

impl Codec for L1BlockCommitment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode height as u64 for consistency
        #[cfg(feature = "bitcoin")]
        let height_u64 = self.height.to_consensus_u32() as u64;
        #[cfg(not(feature = "bitcoin"))]
        let height_u64 = self.height;

        height_u64.encode(enc)?;
        self.blkid.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let height_u64 = u64::decode(dec)?;

        #[cfg(feature = "bitcoin")]
        let height = absolute::Height::from_consensus(height_u64 as u32)
            .map_err(|_| CodecError::MalformedField("L1BlockCommitment.height"))?;
        #[cfg(not(feature = "bitcoin"))]
        let height = height_u64 as u32;

        let blkid = L1BlockId::decode(dec)?;
        Ok(Self { height, blkid })
    }
}

// Custom serde implementation to maintain backward compatibility with u64 JSON
#[cfg(feature = "bitcoin")]
impl Serialize for L1BlockCommitment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use ser::SerializeStruct;
        let mut state = serializer.serialize_struct("L1BlockCommitment", 2)?;
        let height_u64 = self.height.to_consensus_u32() as u64;
        state.serialize_field("height", &height_u64)?;
        state.serialize_field("blkid", &self.blkid)?;
        state.end()
    }
}

#[cfg(feature = "bitcoin")]
impl<'de> Deserialize<'de> for L1BlockCommitment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct L1BlockCommitmentVisitor;

        impl<'de> de::Visitor<'de> for L1BlockCommitmentVisitor {
            type Value = L1BlockCommitment;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("struct L1BlockCommitment or tuple (u64, L1BlockId)")
            }

            // Support struct format (JSON, human-readable formats)
            fn visit_map<V>(self, mut map: V) -> Result<L1BlockCommitment, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut height: Option<u64> = None;
                let mut blkid: Option<L1BlockId> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "height" => {
                            height = Some(map.next_value()?);
                        }
                        "blkid" => {
                            blkid = Some(map.next_value()?);
                        }
                        _ => {
                            let _: de::IgnoredAny = map.next_value()?;
                        }
                    }
                }

                let height_u64 = height.ok_or_else(|| de::Error::missing_field("height"))?;
                let blkid = blkid.ok_or_else(|| de::Error::missing_field("blkid"))?;

                let height = absolute::Height::from_consensus(height_u64 as u32).map_err(|e| {
                    de::Error::custom(format!("invalid block height {}: {}", height_u64, e))
                })?;

                Ok(L1BlockCommitment { height, blkid })
            }

            // Support tuple format (bincode, compact binary formats)
            fn visit_seq<A>(self, mut seq: A) -> Result<L1BlockCommitment, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let height_u64: u64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let blkid: L1BlockId = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                let height = absolute::Height::from_consensus(height_u64 as u32).map_err(|e| {
                    de::Error::custom(format!("invalid block height {}: {}", height_u64, e))
                })?;

                Ok(L1BlockCommitment { height, blkid })
            }
        }

        // For human-readable formats (JSON), use deserialize_any to support both struct and tuple
        // For binary formats (bincode), use deserialize_tuple for backward compatibility
        if deserializer.is_human_readable() {
            deserializer.deserialize_any(L1BlockCommitmentVisitor)
        } else {
            // Bincode doesn't support deserialize_any, so we use deserialize_tuple
            deserializer.deserialize_tuple(2, L1BlockCommitmentVisitor)
        }
    }
}

impl Default for L1BlockCommitment {
    fn default() -> Self {
        Self {
            #[cfg(feature = "bitcoin")]
            height: absolute::Height::ZERO,
            #[cfg(not(feature = "bitcoin"))]
            height: 0,
            blkid: L1BlockId::default(),
        }
    }
}

impl fmt::Display for L1BlockCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Show first 2 and last 2 bytes of block ID (4 hex chars each)
        let blkid_bytes = self.blkid.as_ref();
        let first_2 = &blkid_bytes[..2];
        let last_2 = &blkid_bytes[30..];

        let mut first_hex = [0u8; 4];
        let mut last_hex = [0u8; 4];
        hex::encode_to_slice(first_2, &mut first_hex)
            .expect("Failed to encode first 2 bytes to hex");
        hex::encode_to_slice(last_2, &mut last_hex).expect("Failed to encode last 2 bytes to hex");

        #[cfg(feature = "bitcoin")]
        let height_display = self.height.to_consensus_u32();
        #[cfg(not(feature = "bitcoin"))]
        let height_display = self.height;

        write!(
            f,
            "{}@{}..{}",
            height_display,
            str::from_utf8(&first_hex)
                .expect("Failed to convert first 2 hex bytes to UTF-8 string"),
            str::from_utf8(&last_hex).expect("Failed to convert last 2 hex bytes to UTF-8 string")
        )
    }
}

impl L1BlockCommitment {
    /// Create a new L1 block commitment from a u64 height.
    ///
    /// Returns `None` if the height is invalid (greater than u32::MAX) when bitcoin feature is
    /// enabled.
    pub fn from_height_u64(height: u64, blkid: L1BlockId) -> Option<Self> {
        #[cfg(feature = "bitcoin")]
        {
            let height = absolute::Height::from_consensus(height as u32).ok()?;
            Some(Self { height, blkid })
        }
        #[cfg(not(feature = "bitcoin"))]
        {
            Some(Self {
                height: height as u32,
                blkid,
            })
        }
    }

    /// Get the block height as u64 for compatibility.
    pub fn height_u64(&self) -> u64 {
        #[cfg(feature = "bitcoin")]
        {
            self.height.to_consensus_u32() as u64
        }
        #[cfg(not(feature = "bitcoin"))]
        {
            self.height as u64
        }
    }

    /// Get the block ID.
    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }
}

#[cfg(feature = "bitcoin")]
impl L1BlockCommitment {
    /// Create a new L1 block commitment.
    ///
    /// # Arguments
    /// * `height` - The block height
    /// * `blkid` - The block ID
    pub fn new(height: absolute::Height, blkid: L1BlockId) -> Self {
        Self { height, blkid }
    }

    /// Get the block height.
    pub fn height(&self) -> absolute::Height {
        self.height
    }
}

#[cfg(feature = "bitcoin")]
impl Arbitrary<'_> for L1BlockCommitment {
    fn arbitrary(u: &mut Unstructured<'_>) -> ArbitraryResult<Self> {
        // Heights must be less than 500_000_000 (LOCK_TIME_THRESHOLD)
        let h = u32::arbitrary(u)? % 500_000_000;
        let height =
            absolute::Height::from_consensus(h).map_err(|_| ArbitraryError::IncorrectFormat)?;
        let blkid = L1BlockId::arbitrary(u)?;
        Ok(Self { height, blkid })
    }
}

#[cfg(not(feature = "bitcoin"))]
impl L1BlockCommitment {
    /// Create a new L1 block commitment.
    ///
    /// # Arguments
    /// * `height` - The block height
    /// * `blkid` - The block ID
    pub fn new(height: u32, blkid: L1BlockId) -> Self {
        Self { height, blkid }
    }

    /// Get the block height.
    pub fn height(&self) -> u32 {
        self.height
    }
}

#[cfg(not(feature = "bitcoin"))]
impl Arbitrary<'_> for L1BlockCommitment {
    fn arbitrary(u: &mut Unstructured<'_>) -> ArbitraryResult<Self> {
        let height = u32::arbitrary(u)?;
        let blkid = L1BlockId::arbitrary(u)?;
        Ok(Self { height, blkid })
    }
}

// Ord/PartialOrd for non-bitcoin (SSZ-generated) case
#[cfg(not(feature = "bitcoin"))]
impl Ord for L1BlockCommitment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.height(), self.blkid()).cmp(&(other.height(), other.blkid()))
    }
}

#[cfg(not(feature = "bitcoin"))]
impl PartialOrd for L1BlockCommitment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Implement `ToOwnedSsz` to convert from SSZ-generated view type to the manual
/// `L1BlockCommitment` struct when the `bitcoin` feature is enabled.
///
/// This enables proper type resolution when using `external_kind: container` in SSZ schemas,
/// allowing other crates to use `strata_identifiers.L1BlockCommitment` directly instead of
/// defining redundant inline types.
#[cfg(feature = "bitcoin")]
impl ToOwnedSsz<L1BlockCommitment> for L1BlockCommitmentRef<'_> {
    fn to_owned(&self) -> L1BlockCommitment {
        let height_u32 = self.height().expect("valid L1BlockCommitmentRef");
        let blkid = self.blkid().expect("valid L1BlockCommitmentRef");

        let height = absolute::Height::from_consensus(height_u32)
            .expect("valid height from SSZ-decoded L1BlockCommitmentRef");
        // L1BlockId is a transparent wrapper around Buf32, both are [u8; 32]
        let blkid = L1BlockId::from(Buf32::from(*blkid.as_ref()));

        L1BlockCommitment::new(height, blkid)
    }
}

// Custom debug implementation to print the block hash in little endian
impl fmt::Debug for L1BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut bytes = self.0.0;
        bytes.reverse();
        let mut buf = [0u8; 64]; // 32 bytes * 2 for hex
        encode_to_slice(bytes, &mut buf).expect("buf: enc hex");
        // SAFETY: hex encoding always produces valid UTF-8
        let hex_str = unsafe { str::from_utf8_unchecked(&buf) };
        f.write_str(hex_str)
    }
}

// Custom display implementation to print the block hash in little endian
impl fmt::Display for L1BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut bytes = self.0.0;
        bytes.reverse();
        let mut buf = [0u8; 64]; // 32 bytes * 2 for hex
        encode_to_slice(bytes, &mut buf).expect("buf: enc hex");
        // SAFETY: hex encoding always produces valid UTF-8
        let hex_str = unsafe { str::from_utf8_unchecked(&buf) };
        f.write_str(hex_str)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod l1_block_id {
        use super::*;

        ssz_proptest!(
            L1BlockId,
            any::<[u8; 32]>().prop_map(Buf32::from),
            transparent_wrapper_of(Buf32, from)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = L1BlockId::from(Buf32::zero());
            let encoded = zero.as_ssz_bytes();
            let decoded = L1BlockId::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }

    mod l1_block_commitment {
        use super::*;

        ssz_proptest!(
            L1BlockCommitment,
            (any::<u32>(), any::<[u8; 32]>()).prop_map(|(height, blkid)| {
                #[cfg(feature = "bitcoin")]
                {
                    let h = height % 500_000_000;
                    let height = absolute::Height::from_consensus(h).unwrap();
                    L1BlockCommitment::new(height, L1BlockId::from(Buf32::from(blkid)))
                }
                #[cfg(not(feature = "bitcoin"))]
                {
                    L1BlockCommitment::new(height, L1BlockId::from(Buf32::from(blkid)))
                }
            })
        );

        #[test]
        fn test_zero_ssz() {
            #[cfg(feature = "bitcoin")]
            let commitment =
                L1BlockCommitment::new(absolute::Height::ZERO, L1BlockId::from(Buf32::zero()));
            #[cfg(not(feature = "bitcoin"))]
            let commitment = L1BlockCommitment::new(0, L1BlockId::from(Buf32::zero()));

            let encoded = commitment.as_ssz_bytes();
            let decoded = L1BlockCommitment::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(commitment.height_u64(), decoded.height_u64());
            assert_eq!(commitment.blkid(), decoded.blkid());
        }
    }
}
