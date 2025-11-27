//! Types relating to epoch bookkeeping.
//!
//! An epoch of a range of sequential blocks defined by the terminal block of
//! the epoch going back to (but not including) the terminal block of a previous
//! epoch.  This uniquely identifies the epoch's final state indirectly,
//! although it's possible for conflicting epochs with different terminal blocks
//! to exist in theory, depending on the consensus algorithm.
//!
//! Epochs are *usually* always the same number of slots, but we're not
//! guaranteeing this yet, so we always include both the epoch number and slot
//! number of the terminal block.
//!
//! We also have a sentinel "null" epoch used to refer to the "finalized epoch"
//! as of the genesis block.

use std::fmt;

use borsh::{BorshDeserialize, BorshSerialize};
use const_hex as hex;
use serde::{Deserialize, Serialize, ser::SerializeStruct};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use crate::{
    buf::Buf32,
    ol::OLBlockId,
    ssz_generated::ssz::commitments::{EpochCommitment, OLBlockCommitment},
};

// TODO convert to u32
type RawEpoch = u64;

impl EpochCommitment {
    pub fn new(epoch: RawEpoch, last_slot: u64, last_blkid: OLBlockId) -> Self {
        Self {
            epoch,
            last_slot,
            last_blkid,
        }
    }

    /// Creates a new instance given the terminal block of an epoch and the
    /// epoch index.
    pub fn from_terminal(epoch: RawEpoch, block: OLBlockCommitment) -> Self {
        Self::new(epoch, block.slot(), *block.blkid())
    }

    /// Creates a "null" epoch with 0 slot, epoch 0, and zeroed blkid.
    pub fn null() -> Self {
        Self::new(0, 0, OLBlockId::from(Buf32::zero()))
    }

    pub fn epoch(&self) -> RawEpoch {
        self.epoch
    }

    pub fn last_slot(&self) -> u64 {
        self.last_slot
    }

    pub fn last_blkid(&self) -> &OLBlockId {
        &self.last_blkid
    }

    /// Returns a [`OLBlockCommitment`] for the final block of the epoch.
    pub fn to_block_commitment(&self) -> OLBlockCommitment {
        OLBlockCommitment::new(self.last_slot, self.last_blkid)
    }

    /// Returns if the terminal blkid is zero.  This signifies a special case
    /// for the genesis epoch (0) before the it is completed.
    pub fn is_null(&self) -> bool {
        Buf32::from(self.last_blkid).is_zero()
    }
}

impl Codec for EpochCommitment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.epoch.encode(enc)?;
        self.last_slot.encode(enc)?;
        self.last_blkid.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let epoch = u64::decode(dec)?;
        let last_slot = u64::decode(dec)?;
        let last_blkid = OLBlockId::decode(dec)?;
        Ok(Self {
            epoch,
            last_slot,
            last_blkid,
        })
    }
}

impl fmt::Display for EpochCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Show first 2 and last 2 bytes of block ID (4 hex chars each)
        let blkid_bytes = self.last_blkid().as_ref();
        let first_2 = &blkid_bytes[..2];
        let last_2 = &blkid_bytes[30..];

        let mut first_hex = [0u8; 4];
        let mut last_hex = [0u8; 4];
        hex::encode_to_slice(first_2, &mut first_hex)
            .expect("Failed to encode first 2 bytes to hex");
        hex::encode_to_slice(last_2, &mut last_hex).expect("Failed to encode last 2 bytes to hex");

        // SAFETY: hex always encodes 2->4 bytes
        write!(
            f,
            "{}[{}]@{}..{}",
            self.last_slot(),
            self.epoch(),
            unsafe { std::str::from_utf8_unchecked(&first_hex) },
            unsafe { std::str::from_utf8_unchecked(&last_hex) },
        )
    }
}

// Serde implementations delegate to fields
impl Serialize for EpochCommitment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("EpochCommitment", 3)?;
        state.serialize_field("epoch", &self.epoch)?;
        state.serialize_field("last_slot", &self.last_slot)?;
        state.serialize_field("last_blkid", &self.last_blkid)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for EpochCommitment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Epoch,
            LastSlot,
            LastBlkid,
        }

        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = EpochCommitment;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("struct EpochCommitment")
            }

            fn visit_map<V>(self, mut map: V) -> Result<EpochCommitment, V::Error>
            where
                V: serde::de::MapAccess<'de>,
            {
                let mut epoch = None;
                let mut last_slot = None;
                let mut last_blkid = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Epoch => {
                            if epoch.is_some() {
                                return Err(serde::de::Error::duplicate_field("epoch"));
                            }
                            epoch = Some(map.next_value()?);
                        }
                        Field::LastSlot => {
                            if last_slot.is_some() {
                                return Err(serde::de::Error::duplicate_field("last_slot"));
                            }
                            last_slot = Some(map.next_value()?);
                        }
                        Field::LastBlkid => {
                            if last_blkid.is_some() {
                                return Err(serde::de::Error::duplicate_field("last_blkid"));
                            }
                            last_blkid = Some(map.next_value()?);
                        }
                    }
                }
                let epoch = epoch.ok_or_else(|| serde::de::Error::missing_field("epoch"))?;
                let last_slot =
                    last_slot.ok_or_else(|| serde::de::Error::missing_field("last_slot"))?;
                let last_blkid =
                    last_blkid.ok_or_else(|| serde::de::Error::missing_field("last_blkid"))?;
                Ok(EpochCommitment {
                    epoch,
                    last_slot,
                    last_blkid,
                })
            }
        }

        deserializer.deserialize_struct(
            "EpochCommitment",
            &["epoch", "last_slot", "last_blkid"],
            Visitor,
        )
    }
}

// Borsh implementations are a shim over SSZ - just write/read SSZ bytes directly
impl BorshSerialize for EpochCommitment {
    fn serialize<W: borsh::io::Write>(&self, writer: &mut W) -> borsh::io::Result<()> {
        let ssz_bytes = ssz::Encode::as_ssz_bytes(self);
        writer.write_all(&ssz_bytes)
    }
}

impl BorshDeserialize for EpochCommitment {
    fn deserialize_reader<R: borsh::io::Read>(reader: &mut R) -> borsh::io::Result<Self> {
        // Read exactly the SSZ fixed length
        // This is critical: we must read exactly the fixed length, not all remaining bytes,
        // because EpochCommitment may be nested inside larger Borsh structures.
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

impl<'a> arbitrary::Arbitrary<'a> for EpochCommitment {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            epoch: u.arbitrary()?,
            last_slot: u.arbitrary()?,
            last_blkid: u.arbitrary()?,
        })
    }
}

impl Ord for EpochCommitment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.epoch, self.last_slot, &self.last_blkid).cmp(&(
            other.epoch,
            other.last_slot,
            &other.last_blkid,
        ))
    }
}

impl PartialOrd for EpochCommitment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod epoch_commitment {
        use super::*;

        ssz_proptest!(
            EpochCommitment,
            (any::<u64>(), any::<u64>(), any::<[u8; 32]>()).prop_map(
                |(epoch, last_slot, blkid)| {
                    EpochCommitment::new(epoch, last_slot, OLBlockId::from(Buf32::from(blkid)))
                }
            )
        );

        #[test]
        fn test_zero_ssz() {
            let commitment = EpochCommitment::null();
            let encoded = commitment.as_ssz_bytes();
            let decoded = EpochCommitment::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(commitment.epoch(), decoded.epoch());
            assert_eq!(commitment.last_slot(), decoded.last_slot());
            assert_eq!(commitment.last_blkid(), decoded.last_blkid());
        }
    }
}
