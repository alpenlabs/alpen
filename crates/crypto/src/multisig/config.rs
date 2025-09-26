use std::{collections::HashSet, marker::PhantomData};

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

use crate::multisig::{errors::MultisigError, traits::CryptoScheme};

/// Configuration for a multisignature authority:
/// who can sign (`keys`) and how many of them must sign (`threshold`).
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigConfig<S: CryptoScheme> {
    /// The public keys of all grant-holders authorized to sign.
    pub keys: Vec<S::PubKey>,
    /// The minimum number of keys that must sign to approve an action.
    pub threshold: u8,
    /// Phantom data to carry the crypto scheme type.
    #[borsh(skip)]
    _phantom: PhantomData<S>,
}

/// Represents a change to the multisig configuration:
/// * removes specified members from `remove_members`
/// * adds the specified `add_members`
/// * updates the threshold.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MultisigConfigUpdate<S: CryptoScheme> {
    add_members: Vec<S::PubKey>,
    remove_members: Vec<S::PubKey>,
    new_threshold: u8,
    /// Phantom data to carry the crypto scheme type.
    _phantom: PhantomData<S>,
}

impl<S: CryptoScheme> MultisigConfig<S> {
    /// Create a new multisig configuration.
    ///
    /// # Errors
    ///
    /// Returns `MultisigError` if:
    /// - `DuplicateAddMember`: The keys list contains duplicate members
    /// - `ZeroThreshold`: The threshold is zero
    /// - `InvalidThreshold`: The threshold exceeds the total number of keys
    pub fn try_new(keys: Vec<S::PubKey>, threshold: u8) -> Result<Self, MultisigError> {
        let mut config = MultisigConfig {
            keys: vec![],
            threshold: 0,
            _phantom: PhantomData,
        };
        let update = MultisigConfigUpdate::new(keys, vec![], threshold);
        config.apply_update(&update)?;

        Ok(config)
    }

    pub fn keys(&self) -> &[S::PubKey] {
        &self.keys
    }

    pub fn threshold(&self) -> u8 {
        self.threshold
    }
}

impl<'a, S: CryptoScheme> Arbitrary<'a> for MultisigConfig<S>
where
    S::PubKey: Arbitrary<'a>,
{
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate at least 2 keys, up to a reasonable maximum (e.g., 20)
        let keys_count = u.int_in_range(2..=20)?;
        let mut keys = Vec::with_capacity(keys_count);

        for _ in 0..keys_count {
            keys.push(S::PubKey::arbitrary(u)?);
        }

        // Generate threshold between 1 and the number of keys
        let threshold = u.int_in_range(1..=keys_count as u8)?;

        Ok(Self {
            keys,
            threshold,
            _phantom: PhantomData,
        })
    }
}

impl<S: CryptoScheme> MultisigConfigUpdate<S> {
    /// Creates a new multisig configuration update.
    ///
    /// # Arguments
    ///
    /// * `add_members` - New public keys to add to the configuration
    /// * `remove_members` - Public keys to remove from the configuration
    /// * `new_threshold` - New threshold value
    pub fn new(
        add_members: Vec<S::PubKey>,
        remove_members: Vec<S::PubKey>,
        new_threshold: u8,
    ) -> Self {
        Self {
            add_members,
            remove_members,
            new_threshold,
            _phantom: PhantomData,
        }
    }

    /// Returns the public keys to remove from the configuration.
    pub fn remove_members(&self) -> &[S::PubKey] {
        &self.remove_members
    }

    /// Returns the new members to add.
    pub fn add_members(&self) -> &[S::PubKey] {
        &self.add_members
    }

    /// Returns the new threshold.
    pub fn new_threshold(&self) -> u8 {
        self.new_threshold
    }
}

impl<S: CryptoScheme> BorshSerialize for MultisigConfigUpdate<S>
where
    S::PubKey: BorshSerialize,
{
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.add_members.serialize(writer)?;
        self.remove_members.serialize(writer)?;
        self.new_threshold.serialize(writer)
    }
}

impl<S: CryptoScheme> BorshDeserialize for MultisigConfigUpdate<S>
where
    S::PubKey: BorshDeserialize,
{
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let add_members = Vec::<S::PubKey>::deserialize_reader(reader)?;
        let remove_members = Vec::<S::PubKey>::deserialize_reader(reader)?;
        let new_threshold = u8::deserialize_reader(reader)?;

        Ok(Self {
            add_members,
            remove_members,
            new_threshold,
            _phantom: PhantomData,
        })
    }
}

impl<'a, S: CryptoScheme> Arbitrary<'a> for MultisigConfigUpdate<S>
where
    S::PubKey: Arbitrary<'a>,
{
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let add_members = Vec::<S::PubKey>::arbitrary(u)?;
        let remove_members = Vec::<S::PubKey>::arbitrary(u)?;
        let new_threshold = u8::arbitrary(u)?;

        Ok(Self {
            add_members,
            remove_members,
            new_threshold,
            _phantom: PhantomData,
        })
    }
}

impl<S: CryptoScheme> MultisigConfig<S> {
    /// Validates that an update can be applied to this configuration.
    /// Ensures no duplicate members in the add list, new members don't already exist in the
    /// current configuration, members to remove exist, and the new threshold is within valid
    /// bounds.
    ///
    /// # Errors
    ///
    /// Returns `MultisigError` if:
    /// - `DuplicateAddMember`: The add members list contains duplicate members
    /// - `DuplicateRemoveMember`: The remove members list contains duplicate members
    /// - `MemberAlreadyExists`: A new member already exists in the current configuration
    /// - `MemberNotFound`: A member to remove doesn't exist in the current configuration
    /// - `ZeroThreshold`: New threshold is zero
    /// - `InvalidThreshold`: New threshold exceeds the total number of keys after update
    pub fn validate_update(&self, update: &MultisigConfigUpdate<S>) -> Result<(), MultisigError> {
        let members_to_add: HashSet<<S as CryptoScheme>::PubKey> =
            update.add_members().iter().cloned().collect();
        let members_to_remove: HashSet<<S as CryptoScheme>::PubKey> =
            update.remove_members().iter().cloned().collect();

        // Ensure no duplicate members in the add list
        if members_to_add.len() != update.add_members().len() {
            return Err(MultisigError::DuplicateAddMember);
        }

        // Ensure no duplicate members in the remove list
        if members_to_remove.len() != update.remove_members().len() {
            return Err(MultisigError::DuplicateRemoveMember);
        }

        // Ensure new members don't already exist in current configuration
        if members_to_add.iter().any(|m| self.keys.contains(m)) {
            return Err(MultisigError::MemberAlreadyExists);
        }

        // Ensure new threshold is not zero
        if update.new_threshold() == 0 {
            return Err(MultisigError::ZeroThreshold);
        }

        // Ensure all members to remove exist in current configuration
        for member_to_remove in update.remove_members() {
            if !self.keys.contains(member_to_remove) {
                return Err(MultisigError::MemberNotFound);
            }
        }

        // Ensure new threshold doesn't exceed updated member count
        let updated_size =
            self.keys.len() + update.add_members().len() - update.remove_members().len();

        if (update.new_threshold() as usize) > updated_size {
            return Err(MultisigError::InvalidThreshold {
                threshold: update.new_threshold(),
                total_keys: updated_size,
            });
        }

        Ok(())
    }

    /// Applies an update to this configuration by removing old members, adding new members, and
    /// updating the threshold.
    ///
    /// This method handles member removal by explicitly matching public keys to remove,
    /// ensuring correctness even when there are concurrent configuration updates.
    pub fn apply_update(&mut self, update: &MultisigConfigUpdate<S>) -> Result<(), MultisigError> {
        self.validate_update(update)?;

        // Remove members by explicitly matching public keys
        self.keys
            .retain(|key| !update.remove_members().contains(key));

        // Add new members
        self.keys.extend_from_slice(update.add_members());

        // Update threshold
        self.threshold = update.new_threshold();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use strata_primitives::buf::Buf32;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::multisig::{errors::MultisigError, schemes::SchnorrScheme};

    type TestMultisigConfig = MultisigConfig<SchnorrScheme>;
    type TestMultisigConfigUpdate = MultisigConfigUpdate<SchnorrScheme>;

    fn make_key(id: u8) -> Buf32 {
        Buf32::new([id; 32])
    }

    #[test]
    fn test_borsh_serde() {
        let mut arb = ArbitraryGenerator::new();
        let config: TestMultisigConfig = arb.generate();

        let borsh_serialized_config = borsh::to_vec(&config).unwrap();
        let borsh_deserialized_config: TestMultisigConfig =
            borsh::from_slice(&borsh_serialized_config).unwrap();

        assert_eq!(config, borsh_deserialized_config);
    }

    #[test]
    fn test_new_multisig_config() {
        let k1 = make_key(1);
        let k2 = make_key(2);

        // Try creating config with empty keys
        let err = TestMultisigConfig::try_new(vec![], 2).unwrap_err();
        assert_eq!(
            err,
            MultisigError::InvalidThreshold {
                threshold: 2,
                total_keys: 0
            }
        );

        // Try creating config with 0 threshold
        let err = TestMultisigConfig::try_new(vec![], 0).unwrap_err();
        assert_eq!(err, MultisigError::ZeroThreshold);

        // Try creating config with higher threshold
        let err = TestMultisigConfig::try_new(vec![k1, k2], 3).unwrap_err();
        assert_eq!(
            err,
            MultisigError::InvalidThreshold {
                threshold: 3,
                total_keys: 2
            }
        );

        // Test successful config creation
        let config = TestMultisigConfig::try_new(vec![k1, k2], 1).unwrap();
        assert_eq!(config.keys(), &[k1, k2]);
        assert_eq!(config.threshold(), 1);
    }

    #[test]
    fn test_add_new_members() {
        let k1 = make_key(1);
        let k2 = make_key(2);

        // Initial config: keys = [k1, k2], threshold = 2
        let mut base = TestMultisigConfig::try_new(vec![k1, k2], 2).unwrap();

        // Try to set 0 threshold
        let update = TestMultisigConfigUpdate::new(vec![], vec![], 0);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::ZeroThreshold);

        // Try to add k2 again → should error MemberAlreadyExists
        let update = TestMultisigConfigUpdate::new(vec![k2], vec![], 2);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::MemberAlreadyExists);

        // Try to add k3 twice
        let k3 = make_key(3);
        let update = TestMultisigConfigUpdate::new(vec![k3, k3], vec![], 2);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::DuplicateAddMember);

        // Add k3
        let update = TestMultisigConfigUpdate::new(vec![k3], vec![], 2);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k1, k2, k3]);
    }

    #[test]
    fn test_remove_old_members() {
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);
        let k4 = make_key(4);
        let k5 = make_key(5);
        let k6 = make_key(6);

        // Initial config: keys = [k1, k2, k3, k4, k5, k6], threshold = 2
        let mut base = TestMultisigConfig::try_new(vec![k1, k2, k3, k4, k5, k6], 2).unwrap();

        // Try remove k6 twice
        let update = TestMultisigConfigUpdate::new(vec![], vec![k1, k1], 2);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::DuplicateRemoveMember);

        // Remove k6 and k1
        let update = TestMultisigConfigUpdate::new(vec![], vec![k6, k1], 2);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k2, k3, k4, k5]);

        // Current keys: [k2, k3, k4, k5]
        // Remove k3 and k4
        let update = TestMultisigConfigUpdate::new(vec![], vec![k3, k4], 2);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k2, k5]);

        // Try to remove k3 again (non-existent member)
        let update = TestMultisigConfigUpdate::new(vec![], vec![k3], 2);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::MemberNotFound);
    }

    #[test]
    fn test_threshold() {
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);

        // Initial config: keys = [k1, k2, k3], threshold = 2
        let mut base = TestMultisigConfig::try_new(vec![k1, k2, k3], 2).unwrap();

        // Try setting threshold to 0
        let update = TestMultisigConfigUpdate::new(vec![], vec![], 0);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::ZeroThreshold);

        // Try removing two members
        let update = TestMultisigConfigUpdate::new(vec![], vec![k1, k3], 2);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(
            err,
            MultisigError::InvalidThreshold {
                threshold: 2,
                total_keys: 1
            }
        );

        // Removing first and last member with threshold 1
        let update = TestMultisigConfigUpdate::new(vec![], vec![k1, k3], 1);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k2]);
        assert_eq!(base.threshold(), 1);

        // Try removing the only member without adding any new member
        let update = TestMultisigConfigUpdate::new(vec![], vec![k2], 1);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(
            err,
            MultisigError::InvalidThreshold {
                threshold: 1,
                total_keys: 0
            }
        );

        let k4 = make_key(4);
        let k5 = make_key(5);
        let update = TestMultisigConfigUpdate::new(vec![k4, k5], vec![k2], 2);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k4, k5]);
        assert_eq!(base.threshold(), 2);
    }
}
