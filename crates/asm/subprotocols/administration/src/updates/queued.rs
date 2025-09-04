use std::ops::Deref;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_administration_txs::actions::{UpdateAction, UpdateId};

use crate::updates::delayed::DelayedUpdate;

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct QueuedUpdate(DelayedUpdate);

impl Deref for QueuedUpdate {
    type Target = DelayedUpdate;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl QueuedUpdate {
    pub(crate) fn new(id: UpdateId, action: UpdateAction, current_height: u64, delay: u64) -> Self {
        let activation_height = current_height + delay;
        let delayed_update = DelayedUpdate::new(id, action, activation_height);
        Self(delayed_update)
    }

    pub(crate) fn into_id_and_action(self) -> (UpdateId, UpdateAction) {
        self.0.into_id_and_action()
    }
}
