use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::actions::{UpdateAction, UpdateId};

/// A time-delayed upgrade action with different delay semantics
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct DelayedUpdate<T> {
    id: UpdateId,
    action: UpdateAction,
    activation_height: u64,
    _marker: std::marker::PhantomData<T>,
}

/// Shared implementation
impl<T> DelayedUpdate<T> {
    pub fn new(id: UpdateId, action: UpdateAction, activation_height: u64) -> Self {
        Self {
            id,
            action,
            activation_height,
            _marker: std::marker::PhantomData,
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
