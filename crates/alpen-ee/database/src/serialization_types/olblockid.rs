use borsh::{BorshDeserialize, BorshSerialize};
use strata_identifiers::{Buf32, OLBlockId};

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBOLBlockId(Buf32);

impl From<OLBlockId> for DBOLBlockId {
    fn from(value: OLBlockId) -> Self {
        Self(value.into())
    }
}

impl From<DBOLBlockId> for OLBlockId {
    fn from(value: DBOLBlockId) -> Self {
        value.0.into()
    }
}
