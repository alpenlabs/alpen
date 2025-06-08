use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    multisig::{
        authority::MultisigAuthority,
        config::{MultisigConfig, MultisigConfigUpdate},
    },
    roles::Role,
    txs::updates::id::UpdateId,
    upgrades::{committed::CommittedUpgrade, queued::QueuedUpgrade, scheduled::ScheduledUpgrade},
};

/// Holds the state for the upgrade subprotocol, including the various
/// multisignature authorities and any actions still pending execution.
#[derive(Debug, Clone, Eq, PartialEq, Default, BorshSerialize, BorshDeserialize)]
pub struct UpgradeSubprotoState {
    /// List of configurations for multisignature authorities.
    /// Each entry specifies who the signers are and how many signatures
    /// are required to approve an action.
    multisig_authorities: Vec<MultisigAuthority>,

    /// Actions that can still be cancelled by CancelTx while waiting
    /// for their block countdown to complete.
    queued_upgrades: Vec<QueuedUpgrade>,

    /// Actions that have completed their waiting period and can be
    /// enacted by EnactmentTx, but can no longer be cancelled.
    committed_upgrades: Vec<CommittedUpgrade>,

    /// Actions that will be executed automatically without requiring
    /// an EnactmentTx transaction.
    scheduled_upgrades: Vec<ScheduledUpgrade>,
}

impl UpgradeSubprotoState {
    pub fn get_multisig_config(&self, role: &Role) -> Option<&MultisigConfig> {
        if let Some(authority) = self.multisig_authorities.get((*role as u8) as usize) {
            return Some(authority.config());
        }
        None
    }

    pub fn get_authority(&self, role: &Role) -> Option<&MultisigAuthority> {
        self.multisig_authorities.get((*role as u8) as usize)
    }

    pub fn get_authority_mut(&mut self, role: &Role) -> Option<&mut MultisigAuthority> {
        self.multisig_authorities.get_mut((*role as u8) as usize)
    }

    pub fn update_multisig_config(&mut self, role: Role, update: &MultisigConfigUpdate) {
        if let Some(authority) = self.multisig_authorities.get_mut(role as usize) {
            authority.config_mut().update(update);
        }
    }

    pub fn get_queued_upgrade(&self, target_id: &UpdateId) -> Option<&QueuedUpgrade> {
        self.queued_upgrades
            .iter()
            .find(|action| action.id() == target_id)
    }

    pub fn get_scheduled_upgrade(&self, target_id: &UpdateId) -> Option<&ScheduledUpgrade> {
        self.scheduled_upgrades
            .iter()
            .find(|action| action.id() == target_id)
    }

    pub fn add_queued_upgrade(&mut self, upgrade: QueuedUpgrade) {
        self.queued_upgrades.push(upgrade);
    }

    pub fn add_scheduled_upgrade(&mut self, upgrade: ScheduledUpgrade) {
        self.scheduled_upgrades.push(upgrade);
    }

    pub fn remove_queued_upgrade(&mut self, target_id: &UpdateId) {
        if let Some(idx) = self
            .queued_upgrades
            .iter()
            .position(|action| action.id() == target_id)
        {
            // swap the last element into `idx`, then pop
            self.queued_upgrades.swap_remove(idx);
        }
    }

    pub fn move_committed_upgrade_to_scheduled(&mut self, target_id: &UpdateId) {
        if let Some(idx) = self
            .committed_upgrades
            .iter()
            .position(|action| action.id() == target_id)
        {
            // swap the last element into `idx`, then pop
            let upgrade = self.committed_upgrades.swap_remove(idx);
            let scheduled_upgrade: ScheduledUpgrade = upgrade.into();
            self.scheduled_upgrades.push(scheduled_upgrade);
        }
    }

    pub fn tick_and_move_queued_to_committed(&mut self) {
        self.queued_upgrades.iter_mut().for_each(|action| {
            action.decrement_blocks_remaining();
        });

        // Partition actions: extract actions ready to be committed, keep scheduled ones
        let (ready_to_commit, queued): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queued_upgrades)
            .into_iter()
            .partition(|action| action.blocks_remaining() == 0);

        self.queued_upgrades = queued;

        for action in ready_to_commit {
            let committed_upgrade: CommittedUpgrade = action.into();
            self.committed_upgrades.push(committed_upgrade);
        }
    }

    /// Decrements the block countdown for all scheduled actions and returns any actions
    /// that are now ready for execution (blocks_remaining == 0).
    ///
    /// Ready actions are removed from the scheduled list and returned to the caller
    /// for processing. This should typically be called once per block to advance
    /// the countdown timers and collect executable actions.
    pub fn tick_and_collect_ready_actions(&mut self) -> Vec<ScheduledUpgrade> {
        self.scheduled_upgrades
            .iter_mut()
            .for_each(|action| action.decrement_blocks_remaining());

        // Partition actions: extract ready ones, keep scheduled ones
        let (ready, scheduled): (Vec<_>, Vec<_>) = std::mem::take(&mut self.scheduled_upgrades)
            .into_iter()
            .partition(|action| action.blocks_remaining() == 0);

        self.scheduled_upgrades = scheduled;
        ready
    }
}
