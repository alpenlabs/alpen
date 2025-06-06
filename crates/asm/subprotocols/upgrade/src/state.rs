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
    /// List of configurations for multisignature authorities.
    /// Each entry specifies who the signers are and how many signatures
    /// are required to approve an action.
    multisig_authority: Vec<MultisigConfig>,

    /// List of upgrade actions awaiting execution.
    /// Each element contains a `PendingUpgradeAction`, which carries its
    /// unique identifier and the details of the action to be executed.
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
        let min_required = updated_size.div_ceil(2);

        if (update.new_threshold() as usize) < min_required {
            return Err(MultisigConfigError::InvalidThreshold {
                threshold: update.new_threshold(),
                min_required,
            });
        }

        Ok(())
    }

    pub fn update(&mut self, update: &MultisigConfigUpdate) {
        self.keys.retain(|key| !update.old_members().contains(key));
        self.keys.extend_from_slice(update.new_members());
        self.threshold = update.new_threshold();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(id: u8) -> PubKey {
        PubKey::new(vec![id; 32])
    }

    #[test]
    fn test_validate_update_duplicate_new_member() {
        let role = Role::BridgeAdmin;
        // Initial config: keys = [k1, k2], threshold = 2
        let k1 = make_key(1);
        let k2 = make_key(2);
        let base = MultisigConfig {
            role,
            keys: vec![k1.clone(), k2.clone()],
            threshold: 2,
        };

        // Try to add k2 again → should error MemberAlreadyExists(k2)
        let update = MultisigConfigUpdate::new(vec![k2.clone()], vec![], 2, role);
        let err = base.validate_update(&update).unwrap_err();
        assert_eq!(err, MultisigConfigError::MemberAlreadyExists(k2.clone()));
    }

    #[test]
    fn test_validate_update_missing_old_member() {
        // Initial config: keys = [k1, k2], threshold = 2
        let role = Role::BridgeAdmin;
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);
        let base = MultisigConfig {
            role,
            keys: vec![k1.clone(), k2.clone()],
            threshold: 2,
        };

        // Try to remove k3 (which is not in base.keys) → should error MemberNotFound(k3)
        let update = MultisigConfigUpdate::new(vec![], vec![k3.clone()], 2, role);
        let err = base.validate_update(&update).unwrap_err();
        assert_eq!(err, MultisigConfigError::MemberNotFound(k3.clone()));
    }

    #[test]
    fn test_validate_update_invalid_threshold() {
        // Initial config: keys = [k1, k2, k3, k4], threshold = 3
        let role = Role::BridgeAdmin;
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);
        let k4 = make_key(4);

        let base = MultisigConfig {
            role,
            keys: vec![k1.clone(), k2.clone(), k3.clone(), k4.clone()],
            threshold: 3,
        };

        // Remove k4, add k5 and k6 → updated_size = 5 (since 4 - 1 + 2)
        // min_required = ceil(updated_size / 2) = 3
        // If new_threshold is 2 (< min_required), it should be invalid.
        let k5 = make_key(5);
        let k6 = make_key(6);

        // new_threshold = 2  (invalid, must be > 2)
        let update =
            MultisigConfigUpdate::new(vec![k5.clone(), k6.clone()], vec![k4.clone()], 2, role);
        let err = base.validate_update(&update).unwrap_err();
        assert_eq!(
            err,
            MultisigConfigError::InvalidThreshold {
                threshold: 2,
                min_required: 3,
            }
        );
    }

    #[test]
    fn test_validate_update_success() {
        // Initial config: keys = [k1, k2, k3], threshold = 2
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);

        let role = Role::BridgeAdmin;
        let mut config = MultisigConfig {
            role,
            keys: vec![k1.clone(), k2.clone(), k3.clone()],
            threshold: 2,
        };

        // Remove k3, add k4 and k5 → updated_size = 4 (3 - 1 + 2)
        // min_required = 4 / 2 = 2, so new_threshold must be > 2 (e.g. 3)
        let k4 = make_key(4);
        let k5 = make_key(5);
        let update =
            MultisigConfigUpdate::new(vec![k4.clone(), k5.clone()], vec![k3.clone()], 3, role);

        // First: validate_update should return Ok(())
        assert!(config.validate_update(&update).is_ok());

        // Then, if we actually call `update()`, the resulting config should:
        //   - Keep role the same
        //   - Remove k3 from the keys
        //   - Add k4 and k5
        //   - Have threshold = 3
        config.update(&update);

        // The “new” key‐set should be exactly [k1, k2, k4, k5] (order may matter if you rely on it)
        let expected_keys = vec![k1.clone(), k2.clone(), k4.clone(), k5.clone()];
        assert_eq!(expected_keys, config.keys);

        assert_eq!(config.threshold, 3);
    }
}
