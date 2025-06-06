use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::buf::Buf32;

#[derive(
    Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Hash, BorshSerialize, BorshDeserialize,
)]
pub struct ActionId(Buf32);

// Convert from Buf32 into ActionId
impl From<Buf32> for ActionId {
    fn from(bytes: Buf32) -> Self {
        ActionId(bytes)
    }
}

// Convert from ActionId back into Buf32
impl From<ActionId> for Buf32 {
    fn from(action_id: ActionId) -> Self {
        action_id.0
    }
}

impl ActionId {
    pub fn as_buf32(&self) -> &Buf32 {
        &self.0
    }
}
