use borsh::{BorshDeserialize, BorshSerialize};

use crate::{actions::id::ActionId, txs::updates::UpgradeAction, upgrades::queued::QueuedUpgrade};

/// A committed upgrade action ready for manual execution via EnactmentTx.
/// Cannot be cancelled once in this state.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct CommittedUpgrade {
    id: ActionId,
    action: UpgradeAction,
}

impl From<QueuedUpgrade> for CommittedUpgrade {
    fn from(queued: QueuedUpgrade) -> Self {
        Self {
            id: *queued.id(),
            action: queued.action().clone(),
        }
    }
}

impl CommittedUpgrade {
    pub fn new(id: ActionId, action: UpgradeAction) -> Self {
        Self { id, action }
    }

    // Getters
    pub fn id(&self) -> &ActionId {
        &self.id
    }

    pub fn action(&self) -> &UpgradeAction {
        &self.action
    }
}
