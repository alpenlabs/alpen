use std::collections::HashMap;

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
    multisig_authority: HashMap<Role, MultisigConfig>,

    /// A map from each action’s unique identifier to its corresponding
    /// upgrade action awaiting execution.
    pending_actions: HashMap<ActionId, PendingUpgradeAction>,
}

impl UpgradeSubprotoState {
    pub fn get_multisig_authority_config(&self, role: &Role) -> Option<&MultisigConfig> {
        self.multisig_authority.get(role)
    }

    pub fn add_pending_action(&mut self, id: ActionId, action: PendingUpgradeAction) {
        self.pending_actions.insert(id, action);
    }
}

/// Configuration for a multisignature authority: who the signers are, and
/// how many signatures are required to approve an action.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigConfig {
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
}
