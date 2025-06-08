use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    txs::updates::{UpgradeAction, id::UpdateId},
    upgrades::queued::QueuedUpgrade,
};

/// A committed upgrade action ready for manual execution via EnactmentTx.
/// Cannot be cancelled once in this state.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct CommittedUpgrade {
    id: UpdateId,
    action: UpgradeAction,
}

impl CommittedUpgrade {
    pub fn new(id: UpdateId, action: UpgradeAction) -> Self {
        Self { id, action }
    }

    // Getters
    pub fn id(&self) -> &UpdateId {
        &self.id
    }

    pub fn action(&self) -> &UpgradeAction {
        &self.action
    }

    pub fn into_id_and_action(self) -> (UpdateId, UpgradeAction) {
        (self.id, self.action)
    }
}

impl From<QueuedUpgrade> for CommittedUpgrade {
    fn from(queued: QueuedUpgrade) -> Self {
        let (id, action) = queued.into_id_and_action();
        Self::new(id, action)
    }
}
