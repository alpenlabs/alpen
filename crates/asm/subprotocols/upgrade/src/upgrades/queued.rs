use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    error::UpgradeActionError, roles::StrataProof, txs::updates::UpgradeAction,
    upgrades::delayed::DelayedUpgrade,
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct QueueDelay;

pub type QueuedUpgrade = DelayedUpgrade<QueueDelay>;

pub const ASM_VK_QUEUE_DELAY: u64 = 12_960;
pub const OL_STF_VK_QUEUE_DELAY: u64 = 4_320;

impl TryFrom<UpgradeAction> for QueuedUpgrade {
    type Error = UpgradeActionError;

    fn try_from(action: UpgradeAction) -> Result<Self, Self::Error> {
        let delay = match &action {
            UpgradeAction::VerifyingKey(vk) => match vk.proof_kind() {
                StrataProof::ASM => ASM_VK_QUEUE_DELAY,
                StrataProof::OlStf => OL_STF_VK_QUEUE_DELAY,
            },
            _ => Err(UpgradeActionError::CannotQueue)?,
        };
        let id = action.compute_id();
        Ok(Self::new(id, action, delay))
    }
}
