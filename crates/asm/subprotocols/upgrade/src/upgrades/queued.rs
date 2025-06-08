use borsh::{BorshDeserialize, BorshSerialize};

use crate::{txs::updates::UpgradeAction, upgrades::delayed::DelayedUpgrade};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct QueueDelay;

pub type QueuedUpgrade = DelayedUpgrade<QueueDelay>;

impl From<UpgradeAction> for QueuedUpgrade {
    fn from(action: UpgradeAction) -> Self {
        let id = action.compute_id();
        Self::new(
            id, action, 0, // No delay for queued upgrades
        )
    }
}
