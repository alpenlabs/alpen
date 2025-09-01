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
pub struct QueueDelay;

pub(crate) type QueuedUpdate = DelayedUpdate<QueueDelay>;

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
        Ok(Self::new(id, action, activation_height))
    }
}
