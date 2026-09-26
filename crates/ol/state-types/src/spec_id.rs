//! OL rules version identifier.

use ssz::DecodeError;
use strata_identifiers::{SszDelegate, impl_ssz_via_delegate};
use thiserror::Error;

/// Identifies the OL rules active for an epoch.
///
/// [`Self::V1`] is the genesis rules. The Nth `CheckpointPredicateEnacted` log
/// processed at an epoch terminal activates spec N+1 from the first block of the
/// next epoch. The log carries no spec identifier, so every predicate rotation
/// advances exactly one spec, including a rotation that changes no rules.
///
/// Variants are appended in activation order and must never be removed or
/// renumbered. Ordering follows activation order. The SSZ representation is a
/// single `uint8` equal to the explicit discriminant (`V1` is `1`); unknown values
/// are rejected, never interpreted as genesis rules.
///
/// The OL state root stores spec versions as raw `uint32` values
/// ([`OLRootState`](crate::OLRootState)). [`TryFrom<u32>`] converts them
/// without truncation, and [`From<OLSpecId>`] for [`u32`] produces them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u8)]
pub enum OLSpecId {
    /// Genesis OL rules.
    V1 = 1,
}

impl OLSpecId {
    /// Spec that genesis runs under and that both version fields of the genesis
    /// state name.
    pub const GENESIS: Self = Self::V1;

    /// Returns the next spec in activation order, or `None` if this binary does
    /// not know it.
    ///
    /// Callers processing an enactment must halt when the successor is unknown.
    pub fn successor(self) -> Option<Self> {
        match self {
            Self::V1 => None,
        }
    }
}

/// Error returned when a raw spec version names no spec this binary knows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("unknown OL spec identifier '{0}'")]
pub struct UnknownOLSpecId(u32);

impl UnknownOLSpecId {
    /// Returns the raw value that names no known spec.
    pub fn raw(&self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for OLSpecId {
    type Error = UnknownOLSpecId;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::V1),
            _ => Err(UnknownOLSpecId(value)),
        }
    }
}

impl From<OLSpecId> for u32 {
    fn from(spec: OLSpecId) -> Self {
        spec as u32
    }
}

impl SszDelegate for OLSpecId {
    type Delegate = u8;

    fn into_delegate(self) -> Self::Delegate {
        self as u8
    }

    fn from_delegate(value: Self::Delegate) -> Result<Self, DecodeError> {
        Self::try_from(u32::from(value)).map_err(|err| DecodeError::BytesInvalid(err.to_string()))
    }
}

impl_ssz_via_delegate!(OLSpecId);

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::view::DecodeView;
    use ssz::{Decode, Encode};
    use tree_hash::{Sha256Hasher, TreeHash};

    use super::OLSpecId;

    // Append each spec and its fixed wire value in activation order.
    const KNOWN_SPECS: &[(OLSpecId, u8)] = &[(OLSpecId::V1, 1)];

    proptest! {
        #[test]
        fn test_ssz_round_trip_uses_one_byte(
            index in 0..KNOWN_SPECS.len(),
            prefix in prop::collection::vec(any::<u8>(), 0..=64),
        ) {
            let (spec, wire_value) = KNOWN_SPECS[index];
            let encoded = spec.as_ssz_bytes();

            prop_assert!(<OLSpecId as Encode>::is_ssz_fixed_len());
            prop_assert!(<OLSpecId as Decode>::is_ssz_fixed_len());
            prop_assert_eq!(<OLSpecId as Encode>::ssz_fixed_len(), 1);
            prop_assert_eq!(<OLSpecId as Decode>::ssz_fixed_len(), 1);
            prop_assert_eq!(spec.ssz_bytes_len(), 1);
            prop_assert_eq!(encoded.as_slice(), &[wire_value]);
            prop_assert_eq!(<OLSpecId as Decode>::from_ssz_bytes(&encoded), Ok(spec));
            prop_assert_eq!(<OLSpecId as DecodeView>::from_ssz_bytes(&encoded), Ok(spec));

            let mut buffer = prefix.clone();
            spec.ssz_append(&mut buffer);
            prop_assert_eq!(&buffer[..prefix.len()], prefix.as_slice());
            prop_assert_eq!(&buffer[prefix.len()..], encoded.as_slice());
        }

        #[test]
        fn test_ssz_decoding_accepts_only_known_values(value in any::<u8>()) {
            let expected = KNOWN_SPECS.iter()
                .find_map(|&(spec, wire_value)| (value == wire_value).then_some(spec));

            prop_assert_eq!(<OLSpecId as Decode>::from_ssz_bytes(&[value]).ok(), expected);
            prop_assert_eq!(<OLSpecId as DecodeView>::from_ssz_bytes(&[value]).ok(), expected);
        }

        #[test]
        fn test_ssz_rejects_invalid_lengths(
            index in 0..KNOWN_SPECS.len(),
            trailing in prop::collection::vec(any::<u8>(), 1..=256),
        ) {
            let mut encoded = KNOWN_SPECS[index].0.as_ssz_bytes();
            encoded.extend_from_slice(&trailing);

            for bytes in [&[][..], encoded.as_slice()] {
                prop_assert!(<OLSpecId as Decode>::from_ssz_bytes(bytes).is_err());
                prop_assert!(<OLSpecId as DecodeView>::from_ssz_bytes(bytes).is_err());
            }
        }

        #[test]
        fn test_tree_hash_matches_uint8(index in 0..KNOWN_SPECS.len()) {
            let (spec, wire_value) = KNOWN_SPECS[index];
            prop_assert_eq!(
                spec.tree_hash_root::<Sha256Hasher>(),
                wire_value.tree_hash_root::<Sha256Hasher>()
            );
        }

        #[test]
        fn test_ordering_follows_activation_order(
            left_index in 0..KNOWN_SPECS.len(),
            right_index in 0..KNOWN_SPECS.len(),
        ) {
            let left = KNOWN_SPECS[left_index].0;
            let right = KNOWN_SPECS[right_index].0;
            prop_assert_eq!(left.cmp(&right), left_index.cmp(&right_index));
            prop_assert_eq!(left.partial_cmp(&right), Some(left.cmp(&right)));
        }

        #[test]
        fn test_successor_follows_activation_order(index in 0..KNOWN_SPECS.len()) {
            let spec = KNOWN_SPECS[index].0;
            let expected = KNOWN_SPECS.get(index + 1).map(|&(next, _)| next);
            prop_assert_eq!(spec.successor(), expected);
        }

        #[test]
        fn test_u32_conversion_accepts_only_known_values(value in any::<u32>()) {
            let expected = KNOWN_SPECS.iter()
                .find_map(|&(spec, wire_value)| (value == u32::from(wire_value)).then_some(spec));

            prop_assert_eq!(OLSpecId::try_from(value).ok(), expected);
            if let Some(spec) = expected {
                prop_assert_eq!(u32::from(spec), value);
            }
        }
    }
}
