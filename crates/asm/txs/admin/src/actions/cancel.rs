use arbitrary::Arbitrary;

use crate::actions::UpdateId;

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct CancelAction {
    /// ID of the update that needs to be cancelled.
    target_id: UpdateId,
}

impl CancelAction {
    pub fn new(id: UpdateId) -> Self {
        CancelAction { target_id: id }
    }

    pub fn target_id(&self) -> &UpdateId {
        &self.target_id
    }
}
