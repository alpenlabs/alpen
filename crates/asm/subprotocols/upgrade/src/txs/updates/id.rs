use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::buf::Buf32;

#[derive(
    Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Hash, BorshSerialize, BorshDeserialize,
)]
pub struct UpdateId(Buf32);

// Convert from Buf32 into ActionId
impl From<Buf32> for UpdateId {
    fn from(bytes: Buf32) -> Self {
        UpdateId(bytes)
    }
}

// Convert from ActionId back into Buf32
impl From<UpdateId> for Buf32 {
    fn from(action_id: UpdateId) -> Self {
        action_id.0
    }
}

impl UpdateId {
    pub fn as_buf32(&self) -> &Buf32 {
        &self.0
    }
}
