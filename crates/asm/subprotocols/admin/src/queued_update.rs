use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_txs_admin::actions::{UpdateAction, UpdateId};
use strata_primitives::L1Height;

#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct QueuedUpdate {
    id: UpdateId,
    action: UpdateAction,
    activation_height: L1Height,
}

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
