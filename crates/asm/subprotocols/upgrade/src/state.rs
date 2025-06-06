use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    actions::{ActionId, PendingUpgradeAction, multisig_update::MultisigConfigUpdate},
    crypto::PubKey,
    error::MultisigConfigError,
    roles::Role,
};

/// Holds the state for the upgrade subprotocol, including the various
/// multisignature authorities and any actions still pending execution.
#[derive(Debug, Clone, Eq, PartialEq, Default, BorshSerialize, BorshDeserialize)]
pub struct UpgradeSubprotoState {
    /// Role-specific configuration for a multisignature authority: who the
    /// signers are, and how many signatures are required to approve an action.
    multisig_authority: Vec<MultisigConfig>,

    /// A map from each action’s unique identifier to its corresponding
    /// upgrade action awaiting execution.
    pending_actions: Vec<PendingUpgradeAction>,
}

impl UpgradeSubprotoState {
    pub fn get_multisig_config(&self, role: &Role) -> Option<&MultisigConfig> {
        self.multisig_authority.get((*role as u8) as usize)
    }

    pub fn update_multisig_config(&mut self, update: &MultisigConfigUpdate) {
        if let Some(config) = self
            .multisig_authority
            .get_mut((*update.role() as u8) as usize)
        {
            config.update(update);
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

/// Configuration for a multisignature authority: who the signers are, and
/// how many signatures are required to approve an action.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigConfig {
    pub role: Role,
    /// The public keys of all grant-holders authorized to sign.
    pub keys: Vec<PubKey>,
    /// The minimum number of keys that must sign to approve an action.
    pub threshold: u8,
}

impl MultisigConfig {
    // TODO: add test
    pub fn validate_update(
        &self,
        update: &MultisigConfigUpdate,
    ) -> Result<(), MultisigConfigError> {
        // Ensure no duplicate new members
        if let Some(duplicate) = update.new_members().iter().find(|m| self.keys.contains(*m)) {
            // `duplicate` is a reference to the first member that already exists in `self.keys`.
            return Err(MultisigConfigError::MemberAlreadyExists(duplicate.clone()));
        }

        // Ensure old members exist
        if let Some(missing) = update
            .old_members()
            .iter()
            .find(|m| !self.keys.contains(*m))
        {
            // `missing` is the first member that wasn’t found in `self.keys`
            return Err(MultisigConfigError::MemberNotFound(missing.clone()));
        }

        // Ensure new threshold is strictly greater than half
        let updated_size =
            self.keys.len() + update.new_members().len() - update.old_members().len();
        let min_required = updated_size / 2;

        if update.new_threshold() as usize <= min_required {
            return Err(MultisigConfigError::InvalidThreshold {
                threshold: update.new_threshold(),
                min_required,
            });
        }

        Ok(())
    }

    pub fn update(&self, update: &MultisigConfigUpdate) -> Self {
        let mut new_keys = self.keys.clone();
        new_keys.retain(|key| !update.old_members().contains(key));
        new_keys.extend_from_slice(update.new_members());

        Self {
            role: self.role,
            keys: new_keys,
            threshold: update.new_threshold(),
        }
    }
}
