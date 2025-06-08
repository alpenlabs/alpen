use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::buf::Buf32;

/// A unique identifier for an update
#[derive(Debug, Clone, Copy, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct UpdateId(Buf32);

/// Convert a `Buf32` into an `UpdateId`.
impl From<Buf32> for UpdateId {
    fn from(bytes: Buf32) -> Self {
        UpdateId(bytes)
    }
}

/// Convert an `UpdateId` back into a `Buf32`.
impl From<UpdateId> for Buf32 {
    fn from(action_id: UpdateId) -> Self {
        action_id.0
    }
}

/// Borrow the inner `Buf32` from `&UpdateId`.
impl AsRef<Buf32> for UpdateId {
    fn as_ref(&self) -> &Buf32 {
        &self.0
    }
}
