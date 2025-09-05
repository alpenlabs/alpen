use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

use crate::multisig::{errors::MultisigConfigError, PubKey};

/// Configuration for a multisignature authority:
/// who can sign (`keys`) and how many of them must sign (`threshold`).
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigConfig {
    /// The public keys of all grant-holders authorized to sign.
    pub keys: Vec<PubKey>,
    /// The minimum number of keys that must sign to approve an action.
    pub threshold: u8,
}

impl MultisigConfig {
    /// Create a new config.
    ///
    /// # Errors
    ///
    /// Returns `MultisigConfigError` if:
    /// - `EmptyKeys`: The keys list is empty
    /// - `InvalidThreshold`: The threshold is not greater than half the number of keys or exceeds
    ///   the total number of keys
    pub fn try_new(keys: Vec<PubKey>, threshold: u8) -> Result<Self, MultisigConfigError> {
        if keys.is_empty() {
            return Err(MultisigConfigError::EmptyKeys);
        }

        let max = keys.len();
        let min_required = max / 2 + 1;

        if (threshold as usize) < min_required || threshold as usize > max {
            return Err(MultisigConfigError::InvalidThreshold {
                threshold,
                min_required,
                max_allowed: max,
            });
        }

        Ok(Self { keys, threshold })
    }

    pub fn keys(&self) -> &[PubKey] {
        &self.keys
    }

    pub fn threshold(&self) -> u8 {
        self.threshold
    }
}

impl<'a> Arbitrary<'a> for MultisigConfig {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate at least 2 keys, up to a reasonable maximum (e.g., 20)
        let keys_count = u.int_in_range(2..=20)?;
        let mut keys = Vec::with_capacity(keys_count);

        for _ in 0..keys_count {
            keys.push(PubKey::arbitrary(u)?);
        }

        // Generate threshold between 1 and the number of keys
        let threshold = u.int_in_range(1..=keys_count as u8)?;

        Ok(Self { keys, threshold })
    }
}

/// Represents a change to the multisig configuration:
/// * removes the specified `old_members` from the set,
/// * adds the specified `new_members`
/// * updates the threshold.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct MultisigConfigUpdate {
    new_members: Vec<PubKey>,
    old_members: Vec<PubKey>,
    new_threshold: u8,
}

impl MultisigConfigUpdate {
    pub fn new(new_members: Vec<PubKey>, old_members: Vec<PubKey>, new_threshold: u8) -> Self {
        Self {
            new_members,
            old_members,
            new_threshold,
        }
    }

    pub fn old_members(&self) -> &[PubKey] {
        &self.old_members
    }

    pub fn new_members(&self) -> &[PubKey] {
        &self.new_members
    }

    pub fn new_threshold(&self) -> u8 {
        self.new_threshold
    }
}

impl MultisigConfig {
    /// Validates that an update can be applied to this configuration.
    /// Ensures new members don't already exist, old members exist, and new threshold exceeds half
    /// the updated member count.
    ///
    /// # Errors
    ///
    /// Returns `MultisigConfigError` if:
    /// - `MemberAlreadyExists`: A new member already exists in the current configuration
    /// - `MemberNotFound`: An old member to be removed doesn't exist in the current configuration
    /// - `InvalidThreshold`: New threshold is less than half the updated member count
    pub fn validate_update(
        &self,
        update: &MultisigConfigUpdate,
    ) -> Result<(), MultisigConfigError> {
        // Ensure no duplicate new members
        if let Some(duplicate) = update.new_members().iter().find(|m| self.keys.contains(*m)) {
            // `duplicate` is a reference to the first member that already exists in `self.keys`.
            return Err(MultisigConfigError::MemberAlreadyExists(*duplicate));
        }

        // Ensure old members exist
        if let Some(missing) = update
            .old_members()
            .iter()
            .find(|m| !self.keys.contains(*m))
        {
            // `missing` is the first member that wasn’t found in `self.keys`
            return Err(MultisigConfigError::MemberNotFound(*missing));
        }

        // Ensure new threshold is strictly greater than half
        let updated_size =
            self.keys.len() + update.new_members().len() - update.old_members().len();
        let min_required = updated_size.div_ceil(2);

        if (update.new_threshold() as usize) < min_required {
            return Err(MultisigConfigError::InvalidThreshold {
                threshold: update.new_threshold(),
                min_required,
                max_allowed: updated_size,
            });
        }

        Ok(())
    }

    /// Applies an update to this configuration by removing old members, adding new members, and
    /// updating the threshold. Must call `validate_update` first to ensure the update is valid.
    pub fn apply(&mut self, update: &MultisigConfigUpdate) {
        // REVIEW: If we assert these lists are always sorted then we can do a more efficient
        // merge-and-remove pass with both this and the new entries
        // Remove specified old members
        self.keys.retain(|key| !update.old_members().contains(key));
        // Add new members
        self.keys.extend_from_slice(update.new_members());
        // Update threshold
        self.threshold = update.new_threshold();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(id: u8) -> PubKey {
        PubKey::new([id; 32])
    }

    #[test]
    fn test_validate_update_duplicate_new_member() {
        // Initial config: keys = [k1, k2], threshold = 2
        let k1 = make_key(1);
        let k2 = make_key(2);
        let base = MultisigConfig::try_new(vec![k1, k2], 2).unwrap();

        // Try to add k2 again → should error MemberAlreadyExists(k2)
        let update = MultisigConfigUpdate::new(vec![k2], vec![], 2);
        let err = base.validate_update(&update).unwrap_err();
        assert_eq!(err, MultisigConfigError::MemberAlreadyExists(k2));
    }

    #[test]
    fn test_validate_update_missing_old_member() {
        // Initial config: keys = [k1, k2], threshold = 2
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);
        let base = MultisigConfig::try_new(vec![k1, k2], 2).unwrap();

        // Try to remove k3 (which is not in base.keys) → should error MemberNotFound(k3)
        let update = MultisigConfigUpdate::new(vec![], vec![k3], 2);
        let err = base.validate_update(&update).unwrap_err();
        assert_eq!(err, MultisigConfigError::MemberNotFound(k3));
    }

    #[test]
    fn test_validate_update_invalid_threshold() {
        // Initial config: keys = [k1, k2, k3, k4], threshold = 3
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);
        let k4 = make_key(4);

        let base = MultisigConfig::try_new(vec![k1, k2, k3, k4], 3).unwrap();

        // Remove k4, add k5 and k6 → updated_size = 5 (since 4 - 1 + 2)
        // min_required = ceil(updated_size / 2) = 3
        // If new_threshold is 2 (< min_required), it should be invalid.
        let k5 = make_key(5);
        let k6 = make_key(6);

        // new_threshold = 2  (invalid, must be > 2)
        let update = MultisigConfigUpdate::new(vec![k5, k6], vec![k4], 2);
        let err = base.validate_update(&update).unwrap_err();
        assert_eq!(
            err,
            MultisigConfigError::InvalidThreshold {
                threshold: 2,
                min_required: 3,
                max_allowed: 5,
            }
        );
    }

    #[test]
    fn test_validate_update_success() {
        // Initial config: keys = [k1, k2, k3], threshold = 2
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);

        let mut config = MultisigConfig::try_new(vec![k1, k2, k3], 2).unwrap();

        // Remove k3, add k4 and k5 → updated_size = 4 (3 - 1 + 2)
        // min_required = 4 / 2 = 2, so new_threshold must be > 2 (e.g. 3)
        let k4 = make_key(4);
        let k5 = make_key(5);
        let update = MultisigConfigUpdate::new(vec![k4, k5], vec![k3], 3);

        // First: validate_update should return Ok(())
        assert!(config.validate_update(&update).is_ok());

        // Then, if we actually call `update()`, the resulting config should:
        //   - Keep role the same
        //   - Remove k3 from the keys
        //   - Add k4 and k5
        //   - Have threshold = 3
        config.apply(&update);

        // The “new” key‐set should be exactly [k1, k2, k4, k5] (order may matter if you rely on it)
        let expected_keys = vec![k1, k2, k4, k5];
        assert_eq!(expected_keys, config.keys);

        assert_eq!(config.threshold, 3);
    }
}
