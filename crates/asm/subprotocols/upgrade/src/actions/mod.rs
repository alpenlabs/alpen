pub mod cancel;
pub mod id;
pub mod upgrades;

use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    actions::{id::ActionId, upgrades::UpgradeAction},
    roles::Role,
};

/// A pending upgrade action that will be triggered after a specified number
/// of Bitcoin blocks unless cancelled by a CancelTx.
///
/// The `blocks_remaining` counter is decremented by one for each new Bitcoin
/// block; when it reaches zero, the specified `upgrade` is automatically
/// enacted.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct PendingUpgradeAction {
    id: ActionId,
    action: UpgradeAction,
    blocks_remaining: u64,
}

impl PendingUpgradeAction {
    pub fn id(&self) -> &ActionId {
        &self.id
    }

    pub fn action(&self) -> &UpgradeAction {
        &self.action
    }

    pub fn role(&self) -> Role {
        self.action.role()
    }

    pub fn blocks_remaining(&self) -> u64 {
        self.blocks_remaining
    }
    pub fn decrement_blocks_remaining(&mut self) {
        if self.blocks_remaining > 0 {
            self.blocks_remaining -= 1;
        }
    }
}

impl From<UpgradeAction> for PendingUpgradeAction {
    fn from(action: UpgradeAction) -> Self {
        let id = action.compute_id();
        let blocks_remaining = action.enactment_delay();

        Self {
            id,
            action,
            blocks_remaining,
        }
    }
}
