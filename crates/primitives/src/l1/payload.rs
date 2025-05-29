//! Types relating to payloads.
//!
//! These types don't care about the *purpose* of the payloads, we only care about what's in them.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};

use crate::{buf::Buf32, hash};

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
    pub const fn dest(&self) -> PayloadDest {
        self.dest
    }

    /// Commitment to the payload.
    pub const fn commitment(&self) -> &Buf32 {
        &self.commitment
    }

    #[allow(dead_code)]
    const fn new(dest: PayloadDest, commitment: Buf32) -> Self {
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
    pub const fn dest(&self) -> PayloadDest {
        self.dest
    }

    /// Commitment to the payload.
    pub const fn commitment(&self) -> &Buf32 {
        &self.commitment
    }

    const fn new(dest: PayloadDest, commitment: Buf32) -> Self {
        Self { dest, commitment }
    }
}

/// Data that is submitted to L1. This can be DA, Checkpoint, etc.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct L1Payload {
    data: Vec<u8>,
    payload_type: L1PayloadType,
}

impl L1Payload {
    pub const fn new(data: Vec<u8>, payload_type: L1PayloadType) -> Self {
        Self { data, payload_type }
    }

    pub const fn new_checkpoint(data: Vec<u8>) -> Self {
        Self::new(data, L1PayloadType::Checkpoint)
    }

    pub const fn new_da(data: Vec<u8>) -> Self {
        Self::new(data, L1PayloadType::Da)
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub const fn payload_type(&self) -> &L1PayloadType {
        &self.payload_type
    }

    pub fn hash(&self) -> Buf32 {
        let mut buf = self.data.clone();
        buf.extend(borsh::to_vec(&self.payload_type).expect("Could not serialize payload type"));
        hash::raw(&buf)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub enum L1PayloadType {
    Checkpoint,
    Da,
}

/// Intent produced by the EE on a "full" verification, but if we're just
/// verifying a proof we may not have access to this but still want to reason
/// about it.
///
/// These are never stored on-chain.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
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
    pub const fn new(dest: PayloadDest, commitment: Buf32, payload: L1Payload) -> Self {
        Self {
            dest,
            commitment,
            payload,
        }
    }

    /// The target we expect the DA payload to be stored on.
    pub const fn dest(&self) -> PayloadDest {
        self.dest
    }

    /// Commitment to the payload, which might be context-specific. This
    /// is conceptually unrelated to the payload ID that we use for tracking which
    /// payloads we've written in the L1 writer bookkeeping.
    pub const fn commitment(&self) -> &Buf32 {
        &self.commitment
    }

    /// The payload that matches the commitment.
    pub const fn payload(&self) -> &L1Payload {
        &self.payload
    }

    /// Generates the spec from the relevant parts of the payload intent that
    /// uniquely refers to the payload data.
    pub const fn to_spec(&self) -> PayloadSpec {
        PayloadSpec::new(self.dest, self.commitment)
    }
}
