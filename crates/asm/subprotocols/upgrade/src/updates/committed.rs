use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::actions::{UpdateAction, UpdateId};

use crate::updates::queued::QueuedUpdate;

/// A committed upgrade action ready for manual execution via EnactmentTx.
/// Cannot be cancelled once in this state.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct CommittedUpdate {
    id: UpdateId,
    action: UpdateAction,
}

impl CommittedUpdate {
    pub fn new(id: UpdateId, action: UpdateAction) -> Self {
        Self { id, action }
    }

    // Getters
    pub fn id(&self) -> &UpdateId {
        &self.id
    }

    pub fn action(&self) -> &UpdateAction {
        &self.action
    }

    pub fn into_id_and_action(self) -> (UpdateId, UpdateAction) {
        (self.id, self.action)
    }
}

impl From<QueuedUpdate> for CommittedUpdate {
    fn from(queued: QueuedUpdate) -> Self {
        let (id, action) = queued.into_id_and_action();
        Self::new(id, action)
    }
}
