//! Forced inclusion types.
//!
//! This is all stubs now so that we can define data structures later.

use arbitrary::Arbitrary;

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct ForcedInclusion {
    payload: Vec<u8>,
}

impl ForcedInclusion {
    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}
