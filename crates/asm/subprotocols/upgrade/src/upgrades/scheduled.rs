use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    error::UpgradeActionError,
    txs::UpgradeAction,
    upgrades::{committed::CommittedUpgrade, delayed::DelayedUpgrade},
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct ExecutionDelay;

pub type ScheduledUpgrade = DelayedUpgrade<ExecutionDelay>;

pub const MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const OPERATOR_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const SEQUENCER_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const VK_UPDATE_ENACTMENT_DELAY: u64 = 144;

impl TryFrom<UpgradeAction> for ScheduledUpgrade {
    type Error = UpgradeActionError;

    fn try_from(action: UpgradeAction) -> Result<Self, Self::Error> {
        let delay = match &action {
            UpgradeAction::VerifyingKey(_) => Err(UpgradeActionError::CannotSchedule)?,
            UpgradeAction::Multisig(_) => Ok(MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY),
            UpgradeAction::OperatorSet(_) => Ok(OPERATOR_UPDATE_ENACTMENT_DELAY),
            UpgradeAction::Sequencer(_) => Ok(SEQUENCER_UPDATE_ENACTMENT_DELAY),
        }?;
        let id = action.compute_id();

        Ok(Self::new(id, action, delay))
    }
}

impl ScheduledUpgrade {
    pub fn try_new(action: UpgradeAction, current_height: u64) -> Result<Self, UpgradeActionError> {
        let delay = match &action {
            UpgradeAction::VerifyingKey(_) => Err(UpgradeActionError::CannotSchedule)?,
            UpgradeAction::Multisig(_) => Ok(MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY),
            UpgradeAction::OperatorSet(_) => Ok(OPERATOR_UPDATE_ENACTMENT_DELAY),
            UpgradeAction::Sequencer(_) => Ok(SEQUENCER_UPDATE_ENACTMENT_DELAY),
        }?;
        let id = action.compute_id();
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
