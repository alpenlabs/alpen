use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, OLBlockId};

// TODO(trey): we should be able to just use these types directly

#[derive(Copy, Clone, Debug, Eq, PartiialEq, Ord, PartialOrd, Deserialize, Serialize)]
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
