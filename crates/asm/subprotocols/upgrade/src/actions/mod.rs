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
    upgrade: UpgradeAction,
    blocks_remaining: u64,
    role: Role,
}

impl PendingUpgradeAction {
    pub fn new(upgrade: UpgradeAction, blocks_remaining: u64, role: Role) -> Self {
        let id = upgrade.compute_id();
        Self {
            id,
            upgrade,
            blocks_remaining,
            role,
        }
    }

    pub fn role(&self) -> &Role {
        &self.role
    }

    pub fn id(&self) -> &ActionId {
        &self.id
    }

    pub fn upgrade(&self) -> &UpgradeAction {
        &self.upgrade
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
