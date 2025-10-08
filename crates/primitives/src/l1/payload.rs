//! Types relating to payloads.
//!
//! These types don't care about the *purpose* of the payloads, we only care about what's in them.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use strata_l1_txfmt::TagData;

use crate::buf::Buf32;

/// DA destination identifier. This will eventually be used to enable
/// storing payloads on alternative availability schemes.
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    BorshDeserialize,
    BorshSerialize,
    IntoPrimitive,
    TryFromPrimitive,
    Serialize,
    Deserialize,
)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum PayloadDest {
    /// If we expect the DA to be on the L1 chain that we settle to. This is
    /// always the strongest DA layer we have access to.
    L1 = 0,
}

/// Manual `Arbitrary` impl so that we always generate L1 DA if we add future
/// ones that would work in totally different ways.
impl<'a> Arbitrary<'a> for PayloadDest {
    fn arbitrary(_u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::L1)
    }
}

/// Summary of a DA payload to be included on a DA layer. Specifies the target and
/// a commitment to the payload.
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
)]
pub struct BlobSpec {
    /// Target settlement layer we're expecting the DA on.
    dest: PayloadDest,

    /// Commitment to the payload (probably just a hash or a
    /// merkle root) that we expect to see committed to DA.
    commitment: Buf32,
}

impl BlobSpec {
    /// The target we expect the DA payload to be stored on.
    pub fn dest(&self) -> PayloadDest {
        self.dest
    }

    /// Commitment to the payload.
    pub fn commitment(&self) -> &Buf32 {
        &self.commitment
    }

    #[expect(dead_code, reason = "Constructor for testing purposes")]
    fn new(dest: PayloadDest, commitment: Buf32) -> Self {
        Self { dest, commitment }
    }
}

/// Summary of a DA payload to be included on a DA layer. Specifies the target and
/// a commitment to the payload.
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
)]
pub struct PayloadSpec {
    /// Target settlement layer we're expecting the DA on.
    dest: PayloadDest,

    /// Commitment to the payload (probably just a hash or a
    /// merkle root) that we expect to see committed to DA.
    commitment: Buf32,
}

impl PayloadSpec {
    /// The target we expect the DA payload to be stored on.
    pub fn dest(&self) -> PayloadDest {
        self.dest
    }

    /// Commitment to the payload.
    pub fn commitment(&self) -> &Buf32 {
        &self.commitment
    }

    fn new(dest: PayloadDest, commitment: Buf32) -> Self {
        Self { dest, commitment }
    }
}

/// Data that is submitted to L1. This can be DA, Checkpoint, etc.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct L1Payload {
    payload: Vec<u8>,
    tag: TagData,
}

impl L1Payload {
    pub fn new(payload: Vec<u8>, tag: TagData) -> Self {
        Self { payload, tag }
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn tag(&self) -> &TagData {
        &self.tag
    }
}

impl BorshSerialize for L1Payload {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize payload Vec<u8>
        BorshSerialize::serialize(&self.payload, writer)?;

        // Serialize TagData fields
        BorshSerialize::serialize(&self.tag.subproto_id(), writer)?;
        BorshSerialize::serialize(&self.tag.tx_type(), writer)?;
        BorshSerialize::serialize(&self.tag.aux_data().to_vec(), writer)?;

        Ok(())
    }
}

impl BorshDeserialize for L1Payload {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        // Deserialize payload Vec<u8>
        let payload = Vec::<u8>::deserialize_reader(reader)?;

        // Deserialize TagData fields
        let subproto_id = u8::deserialize_reader(reader)?;
        let tx_type = u8::deserialize_reader(reader)?;
        let aux_data = Vec::<u8>::deserialize_reader(reader)?;

        let tag = TagData::new(subproto_id, tx_type, aux_data).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid TagData: {}", e),
            )
        })?;

        Ok(Self { payload, tag })
    }
}

// REVIEW: serde serialize/deserialize is only needed for the strata-dbtool
impl Serialize for L1Payload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("L1Payload", 4)?;
        state.serialize_field("payload", &self.payload)?;
        state.serialize_field("subproto_id", &self.tag.subproto_id())?;
        state.serialize_field("tx_type", &self.tag.tx_type())?;
        state.serialize_field("aux_data", &self.tag.aux_data())?;
        state.end()
    }
}

// REVIEW: serde serialize/deserialize is only needed for the strata-dbtool
impl<'de> Deserialize<'de> for L1Payload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            payload: Vec<u8>,
            subproto_id: u8,
            tx_type: u8,
            aux_data: Vec<u8>,
        }

        Helper::deserialize(deserializer).and_then(|h| {
            TagData::new(h.subproto_id, h.tx_type, h.aux_data)
                .map(|tag| L1Payload {
                    payload: h.payload,
                    tag,
                })
                .map_err(serde::de::Error::custom)
        })
    }
}

impl<'a> arbitrary::Arbitrary<'a> for L1Payload {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let payload = Vec::<u8>::arbitrary(u)?;

        let subproto_id = u8::arbitrary(u)?;
        let tx_type = u8::arbitrary(u)?;
        // Limit aux_data to a reasonable size (max 74 bytes as per TagData)
        let aux_data_len = u.int_in_range(0..=74)?;
        let mut aux_data = Vec::with_capacity(aux_data_len);
        for _ in 0..aux_data_len {
            aux_data.push(u8::arbitrary(u)?);
        }

        let tag = TagData::new(subproto_id, tx_type, aux_data)
            .map_err(|_| arbitrary::Error::IncorrectFormat)?;

        Ok(Self { payload, tag })
    }
}

/// Intent produced by the EE on a "full" verification, but if we're just
/// verifying a proof we may not have access to this but still want to reason
/// about it.
///
/// These are never stored on-chain.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize)]
// TODO: rename this to L1PayloadIntent and remove the dest field
pub struct PayloadIntent {
    /// The destination for this payload.
    dest: PayloadDest,

    /// Commitment to the payload.
    commitment: Buf32,

    /// Blob payload.
    payload: L1Payload,
}

impl PayloadIntent {
    pub fn new(dest: PayloadDest, commitment: Buf32, payload: L1Payload) -> Self {
        Self {
            dest,
            commitment,
            payload,
        }
    }

    /// The target we expect the DA payload to be stored on.
    pub fn dest(&self) -> PayloadDest {
        self.dest
    }

    /// Commitment to the payload, which might be context-specific. This
    /// is conceptually unrelated to the payload ID that we use for tracking which
    /// payloads we've written in the L1 writer bookkeeping.
    pub fn commitment(&self) -> &Buf32 {
        &self.commitment
    }

    /// The payload that matches the commitment.
    pub fn payload(&self) -> &L1Payload {
        &self.payload
    }

    /// Generates the spec from the relevant parts of the payload intent that
    /// uniquely refers to the payload data.
    pub fn to_spec(&self) -> PayloadSpec {
        PayloadSpec::new(self.dest, self.commitment)
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;

    use crate::l1::payload::L1Payload;

    #[test]
    fn test_l1_payload_borsh_roundtrip() {
        let l1_payload: L1Payload = ArbitraryGenerator::new().generate();
        let buf = borsh::to_vec(&l1_payload).unwrap();
        let res: L1Payload = borsh::from_slice(&buf).unwrap();
        assert_eq!(res, l1_payload);
    }

    #[test]
    fn test_l1_payload_serde_roundtrip() {
        let l1_payload: L1Payload = ArbitraryGenerator::new().generate();
        let json = serde_json::to_string(&l1_payload).unwrap();
        let res: L1Payload = serde_json::from_str(&json).unwrap();
        assert_eq!(res, l1_payload);
    }
}
