use std::fmt;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use const_hex as hex;
use serde::{Deserialize, Serialize, ser::SerializeStruct};
use ssz_derive::{Decode, Encode};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use crate::{buf::Buf32, ssz_generated::ssz::commitments::OLBlockCommitment};

/// ID of an OL (Orchestration Layer) block, usually the hash of its root header.
#[derive(
    Copy,
    Clone,
    Eq,
    Default,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Encode,
    Decode,
)]
#[ssz(struct_behaviour = "transparent")]
pub struct OLBlockId(Buf32);

impl_buf_wrapper!(OLBlockId, Buf32, 32);

// Manual TreeHash implementation for transparent wrapper
impl_ssz_transparent_buf32_wrapper!(OLBlockId);

impl OLBlockId {
    /// Returns a dummy blkid that is all zeroes.
    pub fn null() -> Self {
        Self::from(Buf32::zero())
    }

    /// Checks to see if this is the dummy "zero" blkid.
    pub fn is_null(&self) -> bool {
        self.0.is_zero()
    }
}

/// Alias for backward compatibility
pub type L2BlockId = OLBlockId;

impl crate::OLBlockCommitment {
    pub fn new(slot: u64, blkid: OLBlockId) -> Self {
        Self { slot, blkid }
    }

    pub fn null() -> Self {
        Self::new(0, OLBlockId::from(Buf32::zero()))
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn blkid(&self) -> &OLBlockId {
        &self.blkid
    }

    pub fn is_null(&self) -> bool {
        self.slot == 0 && self.blkid.0.is_zero()
    }
}

impl fmt::Display for OLBlockCommitment {
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

        write!(
            f,
            "{}@{}..{}",
            self.slot,
            std::str::from_utf8(&first_hex)
                .expect("Failed to convert first hex bytes to UTF-8 string"),
            std::str::from_utf8(&last_hex)
                .expect("Failed to convert last hex bytes to UTF-8 string")
        )
    }
}

impl Codec for OLBlockCommitment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.slot.encode(enc)?;
        self.blkid.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let slot = u64::decode(dec)?;
        let blkid = OLBlockId::decode(dec)?;
        Ok(Self { slot, blkid })
    }
}

// Serde implementations delegate to fields
impl Serialize for OLBlockCommitment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("OLBlockCommitment", 2)?;
        state.serialize_field("slot", &self.slot)?;
        state.serialize_field("blkid", &self.blkid)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for OLBlockCommitment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Slot,
            Blkid,
        }

        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = OLBlockCommitment;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("struct OLBlockCommitment")
            }

            fn visit_map<V>(self, mut map: V) -> Result<OLBlockCommitment, V::Error>
            where
                V: serde::de::MapAccess<'de>,
            {
                let mut slot = None;
                let mut blkid = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Slot => {
                            if slot.is_some() {
                                return Err(serde::de::Error::duplicate_field("slot"));
                            }
                            slot = Some(map.next_value()?);
                        }
                        Field::Blkid => {
                            if blkid.is_some() {
                                return Err(serde::de::Error::duplicate_field("blkid"));
                            }
                            blkid = Some(map.next_value()?);
                        }
                    }
                }
                let slot = slot.ok_or_else(|| serde::de::Error::missing_field("slot"))?;
                let blkid = blkid.ok_or_else(|| serde::de::Error::missing_field("blkid"))?;
                Ok(OLBlockCommitment { slot, blkid })
            }
        }

        deserializer.deserialize_struct("OLBlockCommitment", &["slot", "blkid"], Visitor)
    }
}

// Borsh implementations are a shim over SSZ - just write/read SSZ bytes directly
impl BorshSerialize for OLBlockCommitment {
    fn serialize<W: borsh::io::Write>(&self, writer: &mut W) -> borsh::io::Result<()> {
        let ssz_bytes = ssz::Encode::as_ssz_bytes(self);
        writer.write_all(&ssz_bytes)
    }
}

impl BorshDeserialize for OLBlockCommitment {
    fn deserialize_reader<R: borsh::io::Read>(reader: &mut R) -> borsh::io::Result<Self> {
        // Read exactly the SSZ fixed length
        // This is critical: we must read exactly the fixed length, not all remaining bytes,
        // because OLBlockCommitment may be nested inside larger Borsh structures.
        let ssz_fixed_len = <Self as ssz::Decode>::ssz_fixed_len();
        let mut ssz_bytes = vec![0u8; ssz_fixed_len];
        reader.read_exact(&mut ssz_bytes)?;
        ssz::Decode::from_ssz_bytes(&ssz_bytes).map_err(|e| {
            borsh::io::Error::new(
                borsh::io::ErrorKind::InvalidData,
                format!("SSZ decode error: {:?}", e),
            )
        })
    }
}

impl<'a> arbitrary::Arbitrary<'a> for OLBlockCommitment {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            slot: u.arbitrary()?,
            blkid: u.arbitrary()?,
        })
    }
}

impl Ord for OLBlockCommitment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.slot, &self.blkid).cmp(&(other.slot, &other.blkid))
    }
}

impl PartialOrd for OLBlockCommitment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Alias for backward compatibility
pub type L2BlockCommitment = OLBlockCommitment;

/// ID of an OL (Orchestration Layer) transaction.
#[derive(
    Copy,
    Clone,
    Eq,
    Default,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Encode,
    Decode,
)]
#[ssz(struct_behaviour = "transparent")]
pub struct OLTxId(Buf32);

impl_buf_wrapper!(OLTxId, Buf32, 32);

crate::impl_ssz_transparent_buf32_wrapper_copy!(OLTxId);

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod ol_block_id {
        use super::*;

        ssz_proptest!(
            OLBlockId,
            any::<[u8; 32]>().prop_map(Buf32::from),
            transparent_wrapper_of(Buf32, from)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = OLBlockId::from(Buf32::zero());
            let encoded = zero.as_ssz_bytes();
            let decoded = OLBlockId::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }

    mod ol_block_commitment {
        use super::*;

        ssz_proptest!(
            OLBlockCommitment,
            (any::<u64>(), any::<[u8; 32]>()).prop_map(|(slot, blkid)| {
                OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from(blkid)))
            })
        );

        #[test]
        fn test_zero_ssz() {
            let commitment = OLBlockCommitment::new(0, OLBlockId::from(Buf32::zero()));
            let encoded = commitment.as_ssz_bytes();
            let decoded = OLBlockCommitment::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(commitment.slot(), decoded.slot());
            assert_eq!(commitment.blkid(), decoded.blkid());
        }
    }

    mod ol_tx_id {
        use super::*;

        ssz_proptest!(
            OLTxId,
            any::<[u8; 32]>().prop_map(Buf32::from),
            transparent_wrapper_of(Buf32, from)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = OLTxId::from(Buf32::zero());
            let encoded = zero.as_ssz_bytes();
            let decoded = OLTxId::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }
}
