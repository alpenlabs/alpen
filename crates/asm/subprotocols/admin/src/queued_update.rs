use arbitrary::{Arbitrary, Unstructured};
use strata_asm_txs_admin::actions::{UpdateAction, UpdateId};
use strata_primitives::L1Height;

pub(crate) use crate::QueuedUpdate;

impl QueuedUpdate {
    pub fn new(id: UpdateId, action: UpdateAction, activation_height: L1Height) -> Self {
        Self {
            id,
            action,
            activation_height,
        }
    }

    pub fn id(&self) -> &UpdateId {
        &self.id
    }

    pub fn action(&self) -> &UpdateAction {
        &self.action
    }

    pub fn activation_height(&self) -> L1Height {
        self.activation_height
    }

    pub fn into_id_and_action(self) -> (UpdateId, UpdateAction) {
        (self.id, self.action)
    }
}

impl<'a> Arbitrary<'a> for QueuedUpdate {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::new(
            UpdateId::arbitrary(u)?,
            UpdateAction::arbitrary(u)?,
            L1Height::arbitrary(u)?,
        ))
    }
}
