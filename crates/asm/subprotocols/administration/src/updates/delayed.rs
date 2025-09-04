use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_administration_txs::actions::{UpdateAction, UpdateId};

/// A time-delayed update action with different delay semantics
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct DelayedUpdate {
    id: UpdateId,
    action: UpdateAction,
    activation_height: u64,
}

/// Shared implementation
impl DelayedUpdate {
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
