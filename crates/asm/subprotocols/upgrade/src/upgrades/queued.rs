use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::actions::{UpdateId, UpgradeAction};
use strata_primitives::roles::ProofType;

use crate::{
    constants::{ASM_VK_QUEUE_DELAY, OL_STF_VK_QUEUE_DELAY},
    error::UpgradeActionError,
    upgrades::delayed::DelayedUpgrade,
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct QueueDelay;

pub type QueuedUpgrade = DelayedUpgrade<QueueDelay>;

impl QueuedUpgrade {
    pub fn try_new(
        id: UpdateId,
        action: UpgradeAction,
        current_height: u64,
    ) -> Result<Self, UpgradeActionError> {
        let delay = match &action {
            UpgradeAction::VerifyingKey(vk) => match vk.kind() {
                ProofType::Asm => ASM_VK_QUEUE_DELAY,
                ProofType::OlStf => OL_STF_VK_QUEUE_DELAY,
            },
            _ => Err(UpgradeActionError::CannotQueue)?,
        };
        let activation_height = current_height + delay;
        Ok(Self::new(id, action, activation_height))
    }
}
