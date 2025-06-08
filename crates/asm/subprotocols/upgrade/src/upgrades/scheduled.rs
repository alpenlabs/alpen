use borsh::{BorshDeserialize, BorshSerialize};

use crate::upgrades::{committed::CommittedUpgrade, delayed::DelayedUpgrade};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct ExecutionDelay;

pub type ScheduledUpgrade = DelayedUpgrade<ExecutionDelay>;

pub const MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const OPERATOR_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const SEQUENCER_UPDATE_ENACTMENT_DELAY: u64 = 2_016;
pub const VK_ENACTMENT_DELAY: u64 = 144;

impl From<CommittedUpgrade> for ScheduledUpgrade {
    fn from(committed: CommittedUpgrade) -> Self {
        Self::new(
            *committed.id(),
            committed.action().clone(),
            0, // No delay for scheduled upgrades
        )
    }
}
