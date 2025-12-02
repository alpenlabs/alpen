//! Configuration types for threshold signing.

use std::{collections::HashSet, num::NonZero};

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

use super::{CompressedPublicKey, ThresholdSignatureError};

/// Maximum number of signers allowed in a threshold configuration.
///
/// This limit is derived from the signer index being a `u8` (0-255),
/// which allows for at most 256 unique signers.
pub const MAX_SIGNERS: usize = 256;

/// Configuration for a threshold signature authority.
///
/// Defines who can sign (`keys`) and how many must sign (`threshold`).
/// The threshold is stored as `NonZero<u8>` to enforce at the type level
/// that it can never be zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThresholdConfig {
    /// Public keys of all authorized signers.
    keys: Vec<CompressedPublicKey>,
    /// Minimum number of signatures required (always >= 1).
    threshold: NonZero<u8>,
}

impl ThresholdConfig {
    /// Create a new threshold configuration.
    ///
    /// # Errors
    ///
    /// Returns `ThresholdSignatureError` if:
    /// - `DuplicateAddMember`: The keys list contains duplicate members
    /// - `ZeroThreshold`: The threshold is zero
    /// - `InvalidThreshold`: The threshold exceeds the total number of keys
    pub fn try_new(
        keys: Vec<CompressedPublicKey>,
        threshold: u8,
    ) -> Result<Self, ThresholdSignatureError> {
        // Validate threshold is non-zero
        let threshold = NonZero::new(threshold).ok_or(ThresholdSignatureError::ZeroThreshold)?;

        let mut config = ThresholdConfig {
            keys: vec![],
            threshold,
        };
        let update = ThresholdConfigUpdate::new(keys, vec![], threshold.get());
        config.apply_update(&update)?;
        Ok(config)
    }

    /// Get the public keys.
    pub fn keys(&self) -> &[CompressedPublicKey] {
        &self.keys
    }

    /// Get the threshold value.
    pub fn threshold(&self) -> u8 {
        self.threshold.get()
    }

    /// Get the number of authorized signers.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Check if there are no authorized signers.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Validates that an update can be applied to this configuration.
    pub fn validate_update(
        &self,
        update: &ThresholdConfigUpdate,
    ) -> Result<(), ThresholdSignatureError> {
        let members_to_add: HashSet<CompressedPublicKey> =
            update.add_members().iter().cloned().collect();
        let members_to_remove: HashSet<CompressedPublicKey> =
            update.remove_members().iter().cloned().collect();

        // Ensure no duplicate members in the add list
        if members_to_add.len() != update.add_members().len() {
            return Err(ThresholdSignatureError::DuplicateAddMember);
        }

        // Ensure no duplicate members in the remove list
        if members_to_remove.len() != update.remove_members().len() {
            return Err(ThresholdSignatureError::DuplicateRemoveMember);
        }

        // Ensure new members don't already exist in current configuration
        if members_to_add.iter().any(|m| self.keys.contains(m)) {
            return Err(ThresholdSignatureError::MemberAlreadyExists);
        }

        // Ensure new threshold is not zero
        if update.new_threshold() == 0 {
            return Err(ThresholdSignatureError::ZeroThreshold);
        }

        // Ensure all members to remove exist in current configuration
        for member_to_remove in update.remove_members() {
            if !self.keys.contains(member_to_remove) {
                return Err(ThresholdSignatureError::MemberNotFound);
            }
        }

        // Ensure new threshold doesn't exceed updated member count
        let updated_size =
            self.keys.len() + update.add_members().len() - update.remove_members().len();

        if (update.new_threshold() as usize) > updated_size {
            return Err(ThresholdSignatureError::InvalidThreshold {
                threshold: update.new_threshold(),
                total_keys: updated_size,
            });
        }

        Ok(())
    }

    /// Applies an update to this configuration.
    pub fn apply_update(
        &mut self,
        update: &ThresholdConfigUpdate,
    ) -> Result<(), ThresholdSignatureError> {
        self.validate_update(update)?;

        // Remove members by matching public keys
        self.keys
            .retain(|key| !update.remove_members().contains(key));

        // Add new members
        self.keys.extend_from_slice(update.add_members());

        // Update threshold - safe because validate_update already checked it's non-zero
        self.threshold =
            NonZero::new(update.new_threshold()).expect("validate_update ensures non-zero");

        Ok(())
    }
}

impl BorshSerialize for ThresholdConfig {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.keys.serialize(writer)?;
        self.threshold.get().serialize(writer)
    }
}

impl BorshDeserialize for ThresholdConfig {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let keys = Vec::<CompressedPublicKey>::deserialize_reader(reader)?;

        // Validate key count doesn't exceed maximum (prevents DoS via large allocations)
        if keys.len() > MAX_SIGNERS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "too many signers: {} exceeds maximum {}",
                    keys.len(),
                    MAX_SIGNERS
                ),
            ));
        }

        let threshold_u8 = u8::deserialize_reader(reader)?;
        let threshold = NonZero::new(threshold_u8).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "threshold cannot be zero")
        })?;
        Ok(Self { keys, threshold })
    }
}

impl std::hash::Hash for CompressedPublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.serialize().hash(state);
    }
}

/// Represents a change to the threshold configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThresholdConfigUpdate {
    add_members: Vec<CompressedPublicKey>,
    remove_members: Vec<CompressedPublicKey>,
    new_threshold: u8,
}

impl ThresholdConfigUpdate {
    /// Creates a new threshold configuration update.
    pub fn new(
        add_members: Vec<CompressedPublicKey>,
        remove_members: Vec<CompressedPublicKey>,
        new_threshold: u8,
    ) -> Self {
        Self {
            add_members,
            remove_members,
            new_threshold,
        }
    }

    /// Returns the public keys to add.
    pub fn add_members(&self) -> &[CompressedPublicKey] {
        &self.add_members
    }

    /// Returns the public keys to remove.
    pub fn remove_members(&self) -> &[CompressedPublicKey] {
        &self.remove_members
    }

    /// Returns the new threshold.
    pub fn new_threshold(&self) -> u8 {
        self.new_threshold
    }

    /// Consume and return the inner components.
    pub fn into_inner(self) -> (Vec<CompressedPublicKey>, Vec<CompressedPublicKey>, u8) {
        (self.add_members, self.remove_members, self.new_threshold)
    }
}

impl BorshSerialize for ThresholdConfigUpdate {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.add_members.serialize(writer)?;
        self.remove_members.serialize(writer)?;
        self.new_threshold.serialize(writer)
    }
}

impl BorshDeserialize for ThresholdConfigUpdate {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let add_members = Vec::<CompressedPublicKey>::deserialize_reader(reader)?;
        let remove_members = Vec::<CompressedPublicKey>::deserialize_reader(reader)?;
        let new_threshold = u8::deserialize_reader(reader)?;

        Ok(Self {
            add_members,
            remove_members,
            new_threshold,
        })
    }
}

impl<'a> Arbitrary<'a> for ThresholdConfigUpdate {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let add_members = Vec::<CompressedPublicKey>::arbitrary(u)?;
        let remove_members = Vec::<CompressedPublicKey>::arbitrary(u)?;
        // Generate a threshold between 1 and max(1, len(add_members))
        let max_threshold = add_members.len().max(1);
        let new_threshold = u.int_in_range(1..=(max_threshold as u8))?;
        Ok(Self {
            add_members,
            remove_members,
            new_threshold,
        })
    }
}

#[cfg(test)]
mod tests {
    use secp256k1::{Secp256k1, SecretKey};

    use super::*;

    fn make_key(seed: u8) -> CompressedPublicKey {
        let secp = Secp256k1::new();
        let mut sk_bytes = [0u8; 32];
        sk_bytes[31] = seed.max(1); // Ensure non-zero
        let sk = SecretKey::from_slice(&sk_bytes).unwrap();
        CompressedPublicKey::from(secp256k1::PublicKey::from_secret_key(&secp, &sk))
    }

    #[test]
    fn test_config_creation() {
        let keys = vec![make_key(1), make_key(2), make_key(3)];
        let config = ThresholdConfig::try_new(keys.clone(), 2).unwrap();

        assert_eq!(config.keys().len(), 3);
        assert_eq!(config.threshold(), 2);
    }

    #[test]
    fn test_config_zero_threshold() {
        let keys = vec![make_key(1)];
        let result = ThresholdConfig::try_new(keys, 0);
        assert!(matches!(
            result,
            Err(ThresholdSignatureError::ZeroThreshold)
        ));
    }

    #[test]
    fn test_config_threshold_exceeds_keys() {
        let keys = vec![make_key(1), make_key(2)];
        let result = ThresholdConfig::try_new(keys, 3);
        assert!(matches!(
            result,
            Err(ThresholdSignatureError::InvalidThreshold { .. })
        ));
    }

    #[test]
    fn test_config_update_add_member() {
        let keys = vec![make_key(1), make_key(2)];
        let mut config = ThresholdConfig::try_new(keys, 2).unwrap();

        let update = ThresholdConfigUpdate::new(vec![make_key(3)], vec![], 2);
        config.apply_update(&update).unwrap();

        assert_eq!(config.keys().len(), 3);
    }

    #[test]
    fn test_config_update_remove_member() {
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);

        let mut config = ThresholdConfig::try_new(vec![k1, k2, k3], 2).unwrap();

        let update = ThresholdConfigUpdate::new(vec![], vec![k2], 2);
        config.apply_update(&update).unwrap();

        assert_eq!(config.keys().len(), 2);
        assert!(!config.keys().contains(&k2));
    }

    #[test]
    fn test_config_borsh_roundtrip() {
        let keys = vec![make_key(1), make_key(2)];
        let config = ThresholdConfig::try_new(keys, 2).unwrap();

        let encoded = borsh::to_vec(&config).unwrap();
        let decoded: ThresholdConfig = borsh::from_slice(&encoded).unwrap();

        assert_eq!(config, decoded);
    }
}
