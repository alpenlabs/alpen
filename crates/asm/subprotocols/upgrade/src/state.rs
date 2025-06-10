use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_proto_upgrade_txs::{
    actions::UpdateId, crypto::multisig_config::MultisigConfigUpdate, roles::Role,
};

use crate::{
    authority::MultisigAuthority,
    upgrades::{committed::CommittedUpgrade, queued::QueuedUpgrade, scheduled::ScheduledUpgrade},
};

/// Holds the state for the upgrade subprotocol, including the various
/// multisignature authorities and any actions still pending execution.
#[derive(Debug, Clone, Eq, PartialEq, Default, BorshSerialize, BorshDeserialize)]
pub struct UpgradeSubprotoState {
    /// List of configurations for multisignature authorities.
    /// Each entry specifies who the signers are and how many signatures
    /// are required to approve an action.
    authorities: Vec<MultisigAuthority>,

    /// Actions that can still be cancelled by CancelTx while waiting
    /// for their block countdown to complete.
    queued: Vec<QueuedUpgrade>,

    /// Actions that have completed their waiting period and can be
    /// enacted by EnactmentTx, but can no longer be cancelled.
    committed: Vec<CommittedUpgrade>,

    /// Actions that will be executed automatically without requiring
    /// an EnactmentTx transaction.
    scheduled: Vec<ScheduledUpgrade>,

    /// UpdateId for the next update
    next_update_id: UpdateId,
}

impl UpgradeSubprotoState {
    /// Get a reference to the authority for the given role.
    pub fn authority(&self, role: Role) -> Option<&MultisigAuthority> {
        self.authorities.get(role as usize)
    }

    /// Get a mutable reference to the authority for the given role.
    pub fn authority_mut(&mut self, role: Role) -> Option<&mut MultisigAuthority> {
        self.authorities.get_mut(role as usize)
    }

    /// Apply a multisig config update for the specified role.
    pub fn apply_multisig_update(&mut self, role: Role, update: &MultisigConfigUpdate) {
        if let Some(auth) = self.authority_mut(role) {
            auth.config_mut().apply(update);
        }
    }

    /// Find a queued upgrade by its ID.
    pub fn find_queued(&self, id: &UpdateId) -> Option<&QueuedUpgrade> {
        self.queued.iter().find(|u| u.id() == id)
    }

    /// Find a committed upgrade by its ID.
    pub fn find_committed(&self, id: &UpdateId) -> Option<&CommittedUpgrade> {
        self.committed.iter().find(|u| u.id() == id)
    }

    /// Find a scheduled upgrade by its ID.
    pub fn find_scheduled(&self, id: &UpdateId) -> Option<&ScheduledUpgrade> {
        self.scheduled.iter().find(|u| u.id() == id)
    }

    /// Queue a new upgrade.
    pub fn enqueue(&mut self, upgrade: QueuedUpgrade) {
        self.queued.push(upgrade);
    }

    /// Schedule an upgrade to run without enactment.
    pub fn schedule(&mut self, upgrade: ScheduledUpgrade) {
        self.scheduled.push(upgrade);
    }

    /// Remove a queued upgrade by swapping it out.
    pub fn remove_queued(&mut self, id: &UpdateId) {
        if let Some(i) = self.queued.iter().position(|u| u.id() == id) {
            self.queued.swap_remove(i);
        }
    }

    /// Commit a scheduled upgrade: move from committed to scheduled.
    pub fn commit_to_schedule(&mut self, id: &UpdateId) {
        if let Some(i) = self.committed.iter().position(|u| u.id() == id) {
            let up = self.committed.swap_remove(i);
            self.scheduled.push(up.into());
        }
    }

    /// Get a reference to the next global update id
    pub fn next_update_id(&self) -> UpdateId {
        self.next_update_id
    }

    /// Increment the next global update id
    pub fn increment_next_update_id(&mut self) {
        self.next_update_id += 1;
    }

    /// Process all queued upgrades and move any whose `activation_height` equals `current_height`
    /// from `queued` into `committed`.
    pub fn process_queued(&mut self, current_height: u64) {
        let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queued)
            .into_iter()
            .partition(|u| u.activation_height() == current_height);
        self.queued = rest;
        self.committed.extend(ready.into_iter().map(Into::into));
    }

    /// Process all queued upgrades and collect those whose `activation_height` equals
    /// `current_height`
    pub fn process_scheduled(&mut self, current_height: u64) -> Vec<ScheduledUpgrade> {
        let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.scheduled)
            .into_iter()
            .partition(|u| u.activation_height() == current_height);
        self.scheduled = rest;
        ready
    }
}
