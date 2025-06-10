use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::actions::{UpdateId, UpgradeAction};

use crate::{
    constants::{
        MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY, OPERATOR_UPDATE_ENACTMENT_DELAY,
        SEQUENCER_UPDATE_ENACTMENT_DELAY, VK_UPDATE_ENACTMENT_DELAY,
    },
    error::UpgradeActionError,
    upgrades::{committed::CommittedUpgrade, delayed::DelayedUpgrade},
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct ExecutionDelay;

pub type ScheduledUpgrade = DelayedUpgrade<ExecutionDelay>;

impl ScheduledUpgrade {
    pub fn try_new(
        id: UpdateId,
        action: UpgradeAction,
        current_height: u64,
    ) -> Result<Self, UpgradeActionError> {
        let delay = match &action {
            UpgradeAction::VerifyingKey(_) => Err(UpgradeActionError::CannotSchedule)?,
            UpgradeAction::Multisig(_) => Ok(MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY),
            UpgradeAction::OperatorSet(_) => Ok(OPERATOR_UPDATE_ENACTMENT_DELAY),
            UpgradeAction::Sequencer(_) => Ok(SEQUENCER_UPDATE_ENACTMENT_DELAY),
        }?;
        let activation_height = current_height + delay;

        Ok(Self::new(id, action, activation_height))
    }
}

impl From<CommittedUpgrade> for ScheduledUpgrade {
    fn from(committed: CommittedUpgrade) -> Self {
        let (id, action) = committed.into_id_and_action();
        let delay = match &action {
            UpgradeAction::VerifyingKey(_) => VK_UPDATE_ENACTMENT_DELAY,
            UpgradeAction::Multisig(_) => MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY,
            UpgradeAction::OperatorSet(_) => OPERATOR_UPDATE_ENACTMENT_DELAY,
            UpgradeAction::Sequencer(_) => SEQUENCER_UPDATE_ENACTMENT_DELAY,
        };
        Self::new(id, action, delay)
    }
}
