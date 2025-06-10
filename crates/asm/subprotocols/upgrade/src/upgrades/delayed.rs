use borsh::{BorshDeserialize, BorshSerialize};

use crate::txs::updates::{UpgradeAction, id::UpdateId};

/// A time-delayed upgrade action with different delay semantics
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct DelayedUpgrade<T> {
    id: UpdateId,
    action: UpgradeAction,
    activation_height: u64,
    _marker: std::marker::PhantomData<T>,
}

/// Shared implementation
impl<T> DelayedUpgrade<T> {
    pub fn new(id: UpdateId, action: UpgradeAction, blocks_remaining: u64) -> Self {
        Self {
            id,
            action,
            activation_height: blocks_remaining,
            _marker: std::marker::PhantomData,
        }
    }

    // Getters
    pub fn id(&self) -> &UpdateId {
        &self.id
    }
    pub fn action(&self) -> &UpgradeAction {
        &self.action
    }
    pub fn activation_height(&self) -> u64 {
        self.activation_height
    }

    pub fn into_id_and_action(self) -> (UpdateId, UpgradeAction) {
        (self.id, self.action)
    }
}
