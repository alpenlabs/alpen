use std::{fmt, io, str};

use arbitrary::{Arbitrary, Error as ArbitraryError, Result as ArbitraryResult, Unstructured};
// Re-export bitcoin types for internal use
#[cfg(feature = "bitcoin")]
pub(crate) use bitcoin::{BlockHash, absolute};
use borsh::{BorshDeserialize, BorshSerialize};
use const_hex as hex;
use hex::encode_to_slice;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

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

/// L1 Block commitment with block height and ID.
///
/// Note: Height is stored as u32 internally in Bitcoin's consensus format,
/// but we serialize/deserialize using a custom implementation to maintain
/// backwards compatibility with existing data (stored as u64).
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct L1BlockCommitment {
    #[cfg(feature = "bitcoin")]
    height: absolute::Height,
    #[cfg(not(feature = "bitcoin"))]
    height: u64,
    blkid: L1BlockId,
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
impl Arbitrary<'_> for L1BlockCommitment {
    fn arbitrary(u: &mut Unstructured<'_>) -> ArbitraryResult<Self> {
        let height = u64::arbitrary(u)?;
        let blkid = L1BlockId::arbitrary(u)?;
        Ok(Self { height, blkid })
    }
}

// Custom Borsh serialization to maintain backward compatibility with u64 storage
impl BorshSerialize for L1BlockCommitment {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        // Serialize height as u64 for backward compatibility
        #[cfg(feature = "bitcoin")]
        let height_u64 = self.height.to_consensus_u32() as u64;
        #[cfg(not(feature = "bitcoin"))]
        let height_u64 = self.height;

        BorshSerialize::serialize(&height_u64, writer)?;
        BorshSerialize::serialize(&self.blkid, writer)?;
        Ok(())
    }
}

impl BorshDeserialize for L1BlockCommitment {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let height_u64 = u64::deserialize_reader(reader)?;

        #[cfg(feature = "bitcoin")]
        let height = absolute::Height::from_consensus(height_u64 as u32).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid block height {height_u64}: {e}"),
            )
        })?;
        #[cfg(not(feature = "bitcoin"))]
        let height = height_u64;

        let blkid = L1BlockId::deserialize_reader(reader)?;
        Ok(Self { height, blkid })
    }
}

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
        let height = height_u64;

        let blkid = L1BlockId::decode(dec)?;
        Ok(Self { height, blkid })
    }
}

// Custom serde implementation to maintain backward compatibility with u64 JSON
impl Serialize for L1BlockCommitment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use ser::SerializeStruct;
        let mut state = serializer.serialize_struct("L1BlockCommitment", 2)?;

        #[cfg(feature = "bitcoin")]
        let height_u64 = self.height.to_consensus_u32() as u64;
        #[cfg(not(feature = "bitcoin"))]
        let height_u64 = self.height;

        state.serialize_field("height", &height_u64)?;
        state.serialize_field("blkid", &self.blkid)?;
        state.end()
    }
}

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

                #[cfg(feature = "bitcoin")]
                let height = absolute::Height::from_consensus(height_u64 as u32).map_err(|e| {
                    de::Error::custom(format!("invalid block height {}: {}", height_u64, e))
                })?;
                #[cfg(not(feature = "bitcoin"))]
                let height = height_u64;

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

                #[cfg(feature = "bitcoin")]
                let height = absolute::Height::from_consensus(height_u64 as u32).map_err(|e| {
                    de::Error::custom(format!("invalid block height {}: {}", height_u64, e))
                })?;
                #[cfg(not(feature = "bitcoin"))]
                let height = height_u64;

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

impl fmt::Debug for L1BlockCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "bitcoin")]
        let height_display = self.height.to_consensus_u32();
        #[cfg(not(feature = "bitcoin"))]
        let height_display = self.height;

        write!(
            f,
            "L1BlockCommitment(height={}, blkid={:?})",
            height_display, self.blkid
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
            Some(Self { height, blkid })
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
            self.height
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

#[cfg(not(feature = "bitcoin"))]
impl L1BlockCommitment {
    /// Create a new L1 block commitment.
    ///
    /// # Arguments
    /// * `height` - The block height
    /// * `blkid` - The block ID
    pub fn new(height: u64, blkid: L1BlockId) -> Self {
        Self { height, blkid }
    }

    /// Get the block height.
    pub fn height(&self) -> u64 {
        self.height
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
