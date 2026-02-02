use std::{io, ops::AddAssign};

use bitcoin::Work;
use borsh::{BorshDeserialize, BorshSerialize};
use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use serde::{Deserialize, Serialize};

/// Serializer for [`Work`] as bytes for rkyv.
struct WorkAsBytes;

impl ArchiveWith<Work> for WorkAsBytes {
    type Archived = Archived<[u8; 32]>;
    type Resolver = Resolver<[u8; 32]>;

    fn resolve_with(field: &Work, resolver: Self::Resolver, out: Place<Self::Archived>) {
        rkyv::Archive::resolve(&field.to_le_bytes(), resolver, out);
    }
}

impl<S> SerializeWith<Work, S> for WorkAsBytes
where
    S: Fallible + ?Sized,
    [u8; 32]: rkyv::Serialize<S>,
{
    fn serialize_with(field: &Work, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        rkyv::Serialize::serialize(&field.to_le_bytes(), serializer)
    }
}

impl<D> DeserializeWith<Archived<[u8; 32]>, Work, D> for WorkAsBytes
where
    D: Fallible + ?Sized,
    Archived<[u8; 32]>: rkyv::Deserialize<[u8; 32], D>,
{
    fn deserialize_with(
        field: &Archived<[u8; 32]>,
        deserializer: &mut D,
    ) -> Result<Work, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(Work::from_le_bytes(bytes))
    }
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct BtcWork(#[rkyv(with = WorkAsBytes)] Work);

impl Default for BtcWork {
    fn default() -> Self {
        Self(Work::from_le_bytes([0u8; 32]))
    }
}

impl From<Work> for BtcWork {
    fn from(work: Work) -> Self {
        Self(work)
    }
}

impl AddAssign for BtcWork {
    fn add_assign(&mut self, rhs: Self) {
        self.0 = self.0 + rhs.0;
    }
}

impl BorshSerialize for BtcWork {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        BorshSerialize::serialize(&self.0.to_le_bytes(), writer)
    }
}

impl BorshDeserialize for BtcWork {
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let bytes = <[u8; 32]>::deserialize_reader(reader)?;
        Ok(Self(Work::from_le_bytes(bytes)))
    }
}

impl<'a> arbitrary::Arbitrary<'a> for BtcWork {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let bytes = <[u8; 32]>::arbitrary(u)?;
        Ok(Self(Work::from_le_bytes(bytes)))
    }
}
