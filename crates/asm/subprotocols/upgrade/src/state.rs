use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    actions::{ActionId, PendingUpgradeAction, multisig_update::MultisigConfigUpdate},
    multisig_config::MultisigConfig,
    roles::Role,
};

/// Holds the state for the upgrade subprotocol, including the various
/// multisignature authorities and any actions still pending execution.
#[derive(Debug, Clone, Eq, PartialEq, Default, BorshSerialize, BorshDeserialize)]
pub struct UpgradeSubprotoState {
    /// List of configurations for multisignature authorities.
    /// Each entry specifies who the signers are and how many signatures
    /// are required to approve an action.
    multisig_authorities: Vec<MultisigAuthority>,

    /// List of upgrade actions awaiting execution.
    /// Each element contains a `PendingUpgradeAction`, which carries its
    /// unique identifier and the details of the action to be executed.
    pending_actions: Vec<PendingUpgradeAction>,
}

impl UpgradeSubprotoState {
    pub fn get_multisig_config(&self, role: &Role) -> Option<&MultisigConfig> {
        if let Some(authority) = self.multisig_authorities.get((*role as u8) as usize) {
            return Some(authority.config());
        }
        None
    }

    pub fn update_multisig_config(&mut self, update: &MultisigConfigUpdate) {
        if let Some(authority) = self
            .multisig_authorities
            .get_mut((*update.role() as u8) as usize)
        {
            authority.config_mut().update(update);
        }
    }

    pub fn add_pending_action(&mut self, action: PendingUpgradeAction) {
        self.pending_actions.push(action);
    }

    pub fn get_pending_action(&self, id: &ActionId) -> Option<&PendingUpgradeAction> {
        self.pending_actions.iter().find(|action| action.id() == id)
    }

    /// Removes pending action by its ID. Returns error if not found.
    pub fn remove_pending_action(&mut self, target_id: &ActionId) {
        if let Some(idx) = self
            .pending_actions
            .iter()
            .position(|action| action.id() == target_id)
        {
            // swap the last element into `idx`, then pop
            self.pending_actions.swap_remove(idx);
        }
    }

    /// Decrements the block countdown for all pending actions and returns any actions
    /// that are now ready for execution (blocks_remaining == 0).
    ///
    /// Ready actions are removed from the pending list and returned to the caller
    /// for processing. This should typically be called once per block to advance
    /// the countdown timers and collect executable actions.
    pub fn tick_and_collect_ready_actions(&mut self) -> Vec<PendingUpgradeAction> {
        self.pending_actions
            .iter_mut()
            .for_each(|action| action.decrement_blocks_remaining());

        // Partition actions: extract ready ones, keep pending ones
        let (ready, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_actions)
            .into_iter()
            .partition(|action| action.blocks_remaining() == 0);

        self.pending_actions = pending;
        ready
    }
}

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigAuthority {
    /// The role of this multisignature authority.
    pub role: Role,
    /// The public keys of all grant-holders authorized to sign.
    pub config: MultisigConfig,
}

impl MultisigAuthority {
    pub fn new(role: Role, config: MultisigConfig) -> Self {
        Self { role, config }
    }

    pub fn role(&self) -> &Role {
        &self.role
    }

    pub fn config(&self) -> &MultisigConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut MultisigConfig {
        &mut self.config
    }
}
