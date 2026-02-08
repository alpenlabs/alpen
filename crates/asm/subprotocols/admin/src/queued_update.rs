use arbitrary::Arbitrary;
use strata_asm_txs_admin::actions::{UpdateAction, UpdateId};

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct QueuedUpdate {
    id: UpdateId,
    action: UpdateAction,
    activation_height: u64,
}

impl QueuedUpdate {
    pub fn new(id: UpdateId, action: UpdateAction, activation_height: u64) -> Self {
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

    pub fn activation_height(&self) -> u64 {
        self.activation_height
    }

    pub fn into_id_and_action(self) -> (UpdateId, UpdateAction) {
        (self.id, self.action)
    }
}
