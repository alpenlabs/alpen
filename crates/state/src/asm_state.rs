//! State bookkeeping necessary for ASM to run.

use borsh::{BorshDeserialize, BorshSerialize};
use rkyv::{
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
    Archived, Place, Resolver,
};
use serde::{Deserialize, Serialize};
use ssz::{decode::Decode, encode::Encode};
use strata_asm_common::{AnchorState, AsmLogEntry};
use strata_asm_stf::AsmStfOutput;

struct AsmLogEntriesAsBytes;

impl ArchiveWith<Vec<AsmLogEntry>> for AsmLogEntriesAsBytes {
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(
        field: &Vec<AsmLogEntry>,
        resolver: Self::Resolver,
        out: Place<Self::Archived>,
    ) {
        let bytes = field.as_ssz_bytes();
        rkyv::Archive::resolve(&bytes, resolver, out);
    }
}

impl<S> SerializeWith<Vec<AsmLogEntry>, S> for AsmLogEntriesAsBytes
where
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(
        field: &Vec<AsmLogEntry>,
        serializer: &mut S,
    ) -> Result<Self::Resolver, S::Error> {
        let bytes = field.as_ssz_bytes();
        rkyv::Serialize::serialize(&bytes, serializer)
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, Vec<AsmLogEntry>, D> for AsmLogEntriesAsBytes
where
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        deserializer: &mut D,
    ) -> Result<Vec<AsmLogEntry>, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(Vec::<AsmLogEntry>::from_ssz_bytes(&bytes).expect("valid ASM log bytes"))
    }
}

/// ASM bookkeping "umbrella" state.
#[derive(
    Debug,
    Clone,
    PartialEq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct AsmState {
    state: AnchorState,
    #[rkyv(with = AsmLogEntriesAsBytes)]
    logs: Vec<AsmLogEntry>,
}

impl AsmState {
    pub fn new(state: AnchorState, logs: Vec<AsmLogEntry>) -> Self {
        Self { state, logs }
    }

    pub fn from_output(output: AsmStfOutput) -> Self {
        Self {
            state: output.state,
            logs: output.manifest.logs.to_vec(),
        }
    }

    pub fn logs(&self) -> &Vec<AsmLogEntry> {
        &self.logs
    }

    pub fn state(&self) -> &AnchorState {
        &self.state
    }
}
