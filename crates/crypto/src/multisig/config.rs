use std::marker::PhantomData;

use arbitrary::Arbitrary;
use bitvec::{slice::BitSlice, vec::BitVec};
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
            return Err(MultisigError::EmptyKeys);
        }

        let total_keys = keys.len();

        if (threshold as usize) < min_required || threshold as usize > max {
            return Err(MultisigConfigError::InvalidThreshold {
                threshold,
                total_keys,
            });
        }

        Ok(Self {
            keys,
            threshold,
            _phantom: PhantomData,
        })
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
    new_members: Vec<S::PubKey>,
    old_members: BitVec,
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
    pub fn new(new_members: Vec<S::PubKey>, old_members: BitVec, new_threshold: u8) -> Self {
        Self {
            new_members,
            old_members,
            new_threshold,
            _phantom: PhantomData,
        }
    }

    /// Returns the bit vector indicating which members to remove by index.
    pub fn old_members(&self) -> &BitSlice {
        &self.old_members
    }

    /// Returns the new members to add.
    pub fn new_members(&self) -> &[S::PubKey] {
        &self.new_members
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
        self.new_members.serialize(writer)?;
        // Convert BitVec to Vec<bool> for serialization
        let old_members_bits: Vec<bool> = self.old_members.iter().map(|b| *b).collect();
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
            new_members,
            old_members,
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
            new_members,
            old_members,
            new_threshold,
            _phantom: PhantomData,
        })
    }
}

impl<S: CryptoScheme> MultisigConfig<S> {
    /// Validates that an update can be applied to this configuration.
    /// Ensures new members don't already exist, indices are valid, and new threshold doesn't
    /// exceed the updated member count.
    ///
    /// # Errors
    ///
    /// Returns `MultisigConfigError` if:
    /// - `MemberAlreadyExists`: A new member already exists in the current configuration
    /// - `InvalidThreshold`: New threshold is less than half the updated member count
    pub fn validate_update(&self, update: &MultisigConfigUpdate<S>) -> Result<(), MultisigError> {
        // Ensure no duplicate new members
        if let Some(duplicate) = update.new_members().iter().find(|m| self.keys.contains(*m)) {
            // `duplicate` is a reference to the first member that already exists in `self.keys`.
            return Err(MultisigConfigError::MemberAlreadyExists(*duplicate));
        }

        // Ensure old member indices don't exceed current key count.
        if update.old_members().len() > self.keys.len() {
            return Err(MultisigError::InvalidThreshold {
                threshold: 0,
                min_required: 0,
                max_allowed: self.keys.len(),.
            });
        }

        // Ensure new threshold doesn't exceed total number of keys.
        let updated_size =
            self.keys.len() + update.new_members().len() - update.old_members().len();

        if (update.new_threshold() as usize) < min_required {
            return Err(MultisigConfigError::InvalidThreshold {
                threshold: update.new_threshold(),
                total_keys: updated_size,
            });
        }

        Ok(())
    }

    /// Applies an update to this configuration by removing old members, adding new members, and
    /// updating the threshold. Must call `validate_update` first to ensure the update is valid.
    pub fn apply(&mut self, update: &MultisigConfigUpdate) {
        // REVIEW: If we assert these lists are always sorted then we can do a more efficient
        // merge-and-remove pass with both this and the new entries
        // Remove members in reverse order to maintain index validity
        let mut indices_to_remove: Vec<usize> = update.old_members().iter_ones().collect();
        indices_to_remove.sort_by(|a, b| b.cmp(a)); // Sort in descending order

        for index in indices_to_remove {
            self.keys.remove(index);
        }
        // Add new members
        self.keys.extend_from_slice(update.new_members());
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
    fn test_bitvec() {
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);
        let k4 = make_key(4);
        let k5 = make_key(5);

        let keys = [k1, k2, k3, k4, k5];

        // Use bitvec to select only k1, k3, and k5 (indices 0, 2, 4)
        let selection = bitvec![1, 0, 1, 0, 1];
        let selected_keys: Vec<Buf32> = selection.iter_ones().map(|index| keys[index]).collect();

        assert_eq!(selected_keys, vec![k1, k3, k5]);
    }

    #[test]
    fn test_validate_update_duplicate_new_member() {
        // Initial config: keys = [k1, k2], threshold = 2
        let k1 = make_key(1);
        let k2 = make_key(2);
        let base = TestMultisigConfig::try_new(vec![k1, k2], 2).unwrap();

        // Try to add k2 again → should error MemberAlreadyExists
        let update = TestMultisigConfigUpdate::new(vec![k2], bitvec![], 2);
        let err = base.validate_update(&update).unwrap_err();
        assert_eq!(err, MultisigError::MemberAlreadyExists);
    }

    #[test]
    fn test_validate_update_bitvec_too_long() {
        // Initial config: keys = [k1, k2], threshold = 2
        let k1 = make_key(1);
        let k2 = make_key(2);
        let base = TestMultisigConfig::try_new(vec![k1, k2], 2).unwrap();

        // Try to use a BitVec longer than the number of keys
        let update = TestMultisigConfigUpdate::new(vec![], bitvec![1, 0, 1], 2);
        let err = base.validate_update(&update).unwrap_err();
        assert!(matches!(err, MultisigError::InvalidThreshold { .. }));
    }

    #[test]
    fn test_validate_update_invalid_threshold() {
        // Initial config: keys = [k1, k2, k3, k4], threshold = 3
        let k1 = make_key(1);
        let k2 = make_key(2);
        let k3 = make_key(3);
        let k4 = make_key(4);

        let base = TestMultisigConfig::try_new(vec![k1, k2, k3, k4], 3).unwrap();

        // Remove k4, add k5 and k6 → updated_size = 5 (since 4 - 1 + 2)
        // If new_threshold is 6 (> updated_size), it should be invalid.
        let k5 = make_key(5);
        let k6 = make_key(6);

        // new_threshold = 2  (invalid, must be > 2)
        // Remove k4 (index 3)
        let update = MultisigConfigUpdate::new(vec![k5, k6], bitvec![0, 0, 0, 1], 2);
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

        let mut config = TestMultisigConfig::try_new(vec![k1, k2, k3], 2).unwrap();

        // Remove k3, add k4 and k5 → updated_size = 4 (3 - 1 + 2)
        // new_threshold can be any value from 1 to 4
        let k4 = make_key(4);
        let k5 = make_key(5);
        let update = TestMultisigConfigUpdate::new(vec![k4, k5], bitvec![0, 0, 1], 3);

        // First: validate_update should return Ok(())
        assert!(config.validate_update(&update).is_ok());

        // Then, if we actually call `update()`, the resulting config should:
        //   - Keep role the same
        //   - Remove k3 from the keys
        //   - Add k4 and k5
        //   - Have threshold = 3
        config.apply_update(&update).unwrap();

        // The "new" key‐set should be exactly [k1, k2, k4, k5] (order may matter if you rely on it)
        let expected_keys = vec![k1, k2, k4, k5];
        assert_eq!(expected_keys, config.keys);

        assert_eq!(config.threshold, 3);
    }
}
