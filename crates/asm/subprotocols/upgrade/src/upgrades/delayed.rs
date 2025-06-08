use borsh::{BorshDeserialize, BorshSerialize};

use crate::{actions::id::ActionId, txs::updates::UpgradeAction};

/// A time-delayed upgrade action with different delay semantics
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct DelayedUpgrade<T> {
    id: ActionId,
    action: UpgradeAction,
    blocks_remaining: u64,
    _marker: std::marker::PhantomData<T>,
}

/// Shared implementation
impl<T> DelayedUpgrade<T> {
    pub fn new(id: ActionId, action: UpgradeAction, blocks_remaining: u64) -> Self {
        Self {
            id,
            action,
            blocks_remaining,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn decrement_blocks_remaining(&mut self) {
        self.blocks_remaining = self.blocks_remaining.saturating_sub(1);
    }

    pub fn is_ready(&self) -> bool {
        self.blocks_remaining == 0
    }

    // Getters
    pub fn id(&self) -> &ActionId {
        &self.id
    }
    pub fn action(&self) -> &UpgradeAction {
        &self.action
    }
    pub fn blocks_remaining(&self) -> u64 {
        self.blocks_remaining
    }
}
