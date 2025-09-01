use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::actions::{UpdateAction, UpdateId};

use crate::{
    constants::{
        MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY, OPERATOR_UPDATE_ENACTMENT_DELAY,
        SEQUENCER_UPDATE_ENACTMENT_DELAY, VK_UPDATE_ENACTMENT_DELAY,
    },
    error::UpdateActionError,
    updates::{committed::CommittedUpdate, delayed::DelayedUpdate},
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct ExecutionDelay;

pub(crate) type ScheduledUpdate = DelayedUpdate<ExecutionDelay>;

impl ScheduledUpdate {
    pub(crate) fn try_new(
        id: UpdateId,
        action: UpdateAction,
        current_height: u64,
    ) -> Result<Self, UpdateActionError> {
        let delay = match &action {
            UpdateAction::VerifyingKey(_) => Err(UpdateActionError::CannotSchedule)?,
            UpdateAction::Multisig(_) => Ok(MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY),
            UpdateAction::OperatorSet(_) => Ok(OPERATOR_UPDATE_ENACTMENT_DELAY),
            UpdateAction::Sequencer(_) => Ok(SEQUENCER_UPDATE_ENACTMENT_DELAY),
        }?;
        let activation_height = current_height + delay;

        Ok(Self::new(id, action, activation_height))
    }
}

impl From<CommittedUpdate> for ScheduledUpdate {
    fn from(committed: CommittedUpdate) -> Self {
        let (id, action) = committed.into_id_and_action();
        let delay = match &action {
            UpdateAction::VerifyingKey(_) => VK_UPDATE_ENACTMENT_DELAY,
            UpdateAction::Multisig(_) => MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY,
            UpdateAction::OperatorSet(_) => OPERATOR_UPDATE_ENACTMENT_DELAY,
            UpdateAction::Sequencer(_) => SEQUENCER_UPDATE_ENACTMENT_DELAY,
        };
        Self::new(id, action, delay)
    }
}
