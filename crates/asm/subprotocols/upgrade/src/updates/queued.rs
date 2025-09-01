use std::ops::Deref;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::actions::{UpdateAction, UpdateId};
use strata_primitives::roles::ProofType;

use crate::{
    constants::{ASM_VK_QUEUE_DELAY, OL_STF_VK_QUEUE_DELAY},
    error::UpdateActionError,
    updates::delayed::DelayedUpdate,
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct QueuedUpdate(DelayedUpdate);

impl Deref for QueuedUpdate {
    type Target = DelayedUpdate;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl QueuedUpdate {
    pub(crate) fn try_new(
        id: UpdateId,
        action: UpdateAction,
        current_height: u64,
    ) -> Result<Self, UpdateActionError> {
        let delay = match &action {
            UpdateAction::VerifyingKey(vk) => match vk.kind() {
                ProofType::Asm => ASM_VK_QUEUE_DELAY,
                ProofType::OlStf => OL_STF_VK_QUEUE_DELAY,
            },
            _ => Err(UpdateActionError::CannotQueue)?,
        };
        let activation_height = current_height + delay;
        let delayed_update = DelayedUpdate::new(id, action, activation_height);
        Ok(Self(delayed_update))
    }

    pub(crate) fn into_id_and_action(self) -> (UpdateId, UpdateAction) {
        self.0.into_id_and_action()
    }
}
