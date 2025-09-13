use std::marker::PhantomData;

use arbitrary::Arbitrary;
use bitvec::{bitvec, prelude::*};
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

impl<S: CryptoScheme> MultisigConfig<S> {
    /// Create a new multisig configuration.
    ///
    /// # Errors
    ///
    /// Returns `MultisigError` if:
    /// - `DuplicateAddMember`: The keys list contains duplicates
    /// - `ZeroThreshold`: The threshold is zero
    /// - `InvalidThreshold`: The threshold exceeds the total number of keys
    pub fn try_new(keys: Vec<S::PubKey>, threshold: u8) -> Result<Self, MultisigError> {
        let mut config = MultisigConfig {
            keys: vec![],
            threshold: 0,
            _phantom: PhantomData,
        };
        let update = MultisigConfigUpdate::new(keys, bitvec![], threshold);
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

/// Represents a change to the multisig configuration:
/// * removes members at indices specified by `old_members` bit vector
/// * adds the specified `new_members`
/// * updates the threshold.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MultisigConfigUpdate<S: CryptoScheme> {
    add_members: Vec<S::PubKey>,
    remove_members: BitVec,
    new_threshold: u8,
    /// Phantom data to carry the crypto scheme type.
    _phantom: PhantomData<S>,
}

impl<S: CryptoScheme> MultisigConfigUpdate<S> {
    /// Creates a new multisig configuration update.
    ///
    /// # Arguments
    ///
    /// * `new_members` - New public keys to add to the configuration
    /// * `old_members` - Bit vector indicating which existing members to remove by index
    /// * `new_threshold` - New threshold value
    pub fn new(add_members: Vec<S::PubKey>, remove_members: BitVec, new_threshold: u8) -> Self {
        Self {
            add_members,
            remove_members,
            new_threshold,
            _phantom: PhantomData,
        }
    }

    /// Returns the bit vector indicating which members to remove by index.
    pub fn remove_members(&self) -> &BitSlice {
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
        // Convert BitVec to Vec<bool> for serialization
        let old_members_bits: Vec<bool> = self.remove_members.iter().map(|b| *b).collect();
        old_members_bits.serialize(writer)?;
        self.new_threshold.serialize(writer)
    }
}

impl<S: CryptoScheme> BorshDeserialize for MultisigConfigUpdate<S>
where
    S::PubKey: BorshDeserialize,
{
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let new_members = Vec::<S::PubKey>::deserialize_reader(reader)?;
        let old_members_bits = Vec::<bool>::deserialize_reader(reader)?;
        let new_threshold = u8::deserialize_reader(reader)?;

        // Convert Vec<bool> back to BitVec
        let old_members = BitVec::from_iter(old_members_bits);

        Ok(Self {
            add_members: new_members,
            remove_members: old_members,
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
        let new_members = Vec::<S::PubKey>::arbitrary(u)?;

        // Generate a reasonable sized bit vector for old members
        let old_members_size = u.int_in_range(0..=20)?;
        let mut old_members = BitVec::with_capacity(old_members_size);
        for _ in 0..old_members_size {
            old_members.push(bool::arbitrary(u)?);
        }

        let new_threshold = u8::arbitrary(u)?;

        Ok(Self {
            add_members: new_members,
            remove_members: old_members,
            new_threshold,
            _phantom: PhantomData,
        })
    }
}

impl<S: CryptoScheme> MultisigConfig<S> {
    /// Validates that an update can be applied to this configuration.
    /// Ensures new members don't already exist, remove indices are valid, and new threshold
    /// is within valid bounds for the updated member count.
    ///
    /// # Errors
    ///
    /// Returns `MultisigError` if:
    /// - `MemberAlreadyExists`: A new member already exists in the current configuration
    /// - `RemovalBitVecTooLong`: The removal bitvec is longer than the current member count
    /// - `InvalidThreshold`: New threshold exceeds the total number of keys after update or is zero
    pub fn validate_update(&self, update: &MultisigConfigUpdate<S>) -> Result<(), MultisigError> {
        let mut members_to_add = update.add_members().to_vec();
        members_to_add.dedup();

        if members_to_add.len() != update.add_members().len() {
            return Err(MultisigError::DuplicateAddMember);
        }

        // Ensure no duplicate members to add
        if members_to_add.iter().any(|m| self.keys.contains(m)) {
            return Err(MultisigError::MemberAlreadyExists);
        }

        // Ensure the removal bitvec doesn't reference invalid member indices
        if update.remove_members().len() > self.keys.len() {
            return Err(MultisigError::RemovalBitVecTooLong {
                bitvec_len: update.remove_members().len(),
                member_count: self.keys.len(),
            });
        }

        if update.new_threshold() == 0 {
            return Err(MultisigError::ZeroThreshold);
        }

        // Ensure new threshold is valid for the updated member count
        let updated_size =
            self.keys.len() + update.add_members().len() - update.remove_members().count_ones();

        if (update.new_threshold() as usize) > updated_size {
            return Err(MultisigError::InvalidThreshold {
                threshold: update.new_threshold(),
                total_keys: updated_size,
            });
        }

        Ok(())
    }

    /// Applies an update to this configuration by removing old members, adding new members, and
    /// updating the threshold. Must call `validate_update` first to ensure the update is valid.
    pub fn apply_update(&mut self, update: &MultisigConfigUpdate<S>) -> Result<(), MultisigError> {
        self.validate_update(update)?;

        // Remove members by index. We must remove in reverse order (highest index first)
        // to prevent index shifting from invalidating subsequent removal indices.
        // When an element is removed, all elements after it shift left by one position.
        let mut indices_to_remove: Vec<usize> = update.remove_members().iter_ones().collect();
        indices_to_remove.reverse(); // Reverse to get descending order
        for index in indices_to_remove {
            self.keys.remove(index);
        }

        // Add new members
        self.keys.extend_from_slice(update.add_members());

        // Update threshold
        self.threshold = update.new_threshold();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bitvec::prelude::*;
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::multisig::{errors::MultisigError, schemes::SchnorrScheme};

    type TestMultisigConfig = MultisigConfig<SchnorrScheme>;
    type TestMultisigConfigUpdate = MultisigConfigUpdate<SchnorrScheme>;

    fn make_key(id: u8) -> Buf32 {
        Buf32::new([id; 32])
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
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![], 0);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::ZeroThreshold);

        // Try to add k2 again → should error MemberAlreadyExists
        let update = TestMultisigConfigUpdate::new(vec![k2], bitvec![], 2);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::MemberAlreadyExists);

        // Try to add k3 twice
        let k3 = make_key(3);
        let update = TestMultisigConfigUpdate::new(vec![k3, k3], bitvec![], 2);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::DuplicateAddMember);

        // Add k3
        let update = TestMultisigConfigUpdate::new(vec![k3], bitvec![], 2);
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

        // Initial config: keys = [k1, k2, k3, k4, k5], threshold = 2
        let mut base = TestMultisigConfig::try_new(vec![k1, k2, k3, k4, k5], 2).unwrap();

        // Try to remove first member using a BitVec longer than the number of keys
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![1, 0, 0, 0, 0, 0], 2);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(
            err,
            MultisigError::RemovalBitVecTooLong {
                bitvec_len: 6,
                member_count: 5
            }
        );

        // Current keys: [k1, k2, k3, k4, k5]
        // Remove the last member
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![0, 0, 0, 0, 1], 2);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k1, k2, k3, k4]);

        // Current keys: [k1, k2, k3, k4]
        // Remove the new first member with smaller bitvec
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![1], 2);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k2, k3, k4]);

        // Current keys: [k2, k3, k4]
        // Try to remove front two members
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![1, 1], 1);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k4]);
    }

    #[test]
    fn test_threshold() {
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);

        // Initial config: keys = [k1, k2, k3], threshold = 2
        let mut base = TestMultisigConfig::try_new(vec![k1, k2, k3], 2).unwrap();

        // Try setting threshold to 0
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![], 0);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::ZeroThreshold);

        // Try removing two members
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![1, 0, 1], 2);
        let err = base.apply_update(&update).unwrap_err();
        assert_eq!(
            err,
            MultisigError::InvalidThreshold {
                threshold: 2,
                total_keys: 1
            }
        );

        // Removing first and last member with threshold 1
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![1, 0, 1], 1);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k2]);
        assert_eq!(base.threshold(), 1);

        // Try removing the only member without adding any new member
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![1], 1);
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
        let update = TestMultisigConfigUpdate::new(vec![k4, k5], bitvec![1], 2);
        base.apply_update(&update).unwrap();
        assert_eq!(base.keys(), &[k4, k5]);
        assert_eq!(base.threshold(), 2);
    }
}
