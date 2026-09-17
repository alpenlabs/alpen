//! OL rules version identifier.

use ssz::DecodeError;
use strata_identifiers::{SszDelegate, impl_ssz_via_delegate};

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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u8)]
pub enum OlSpecId {
    /// Genesis OL rules.
    V1 = 1,
}

impl OlSpecId {
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

impl SszDelegate for OlSpecId {
    type Delegate = u8;

    fn into_delegate(self) -> Self::Delegate {
        self as u8
    }

    fn from_delegate(value: Self::Delegate) -> Result<Self, DecodeError> {
        match value {
            1 => Ok(Self::V1),
            _ => Err(DecodeError::BytesInvalid(format!(
                "unknown OL spec identifier: {value}"
            ))),
        }
    }
}

impl_ssz_via_delegate!(OlSpecId);

#[cfg(test)]
mod tests {
    use ssz::view::DecodeView;
    use ssz::{Decode, Encode};
    use tree_hash::{Sha256Hasher, TreeHash};

    use super::OlSpecId;

    #[test]
    fn test_ssz_round_trip_uses_one_byte() {
        let spec = OlSpecId::V1;
        let encoded = spec.as_ssz_bytes();

        assert!(<OlSpecId as Encode>::is_ssz_fixed_len());
        assert!(<OlSpecId as Decode>::is_ssz_fixed_len());
        assert_eq!(<OlSpecId as Encode>::ssz_fixed_len(), 1);
        assert_eq!(<OlSpecId as Decode>::ssz_fixed_len(), 1);
        assert_eq!(spec.ssz_bytes_len(), 1);
        assert_eq!(encoded, [1]);
        assert_eq!(<OlSpecId as Decode>::from_ssz_bytes(&encoded), Ok(spec));
        assert_eq!(<OlSpecId as DecodeView>::from_ssz_bytes(&encoded), Ok(spec));
    }

    #[test]
    fn test_ssz_rejects_unknown_values() {
        for value in 0..=u8::MAX {
            if value == 1 {
                continue;
            }
            assert!(<OlSpecId as Decode>::from_ssz_bytes(&[value]).is_err());
            assert!(<OlSpecId as DecodeView>::from_ssz_bytes(&[value]).is_err());
        }
    }

    #[test]
    fn test_ssz_rejects_invalid_lengths() {
        for bytes in [&[][..], &[1, 0][..]] {
            assert!(<OlSpecId as Decode>::from_ssz_bytes(bytes).is_err());
            assert!(<OlSpecId as DecodeView>::from_ssz_bytes(bytes).is_err());
        }
    }

    #[test]
    fn test_tree_hash_matches_uint8() {
        assert_eq!(
            OlSpecId::V1.tree_hash_root::<Sha256Hasher>(),
            1_u8.tree_hash_root::<Sha256Hasher>()
        );
    }

    #[test]
    fn test_ordering_follows_activation_order() {
        // Extend this list in activation order when adding a spec.
        let specs = [OlSpecId::V1];
        for (left_index, left) in specs.iter().enumerate() {
            for (right_index, right) in specs.iter().enumerate() {
                assert_eq!(left.cmp(right), left_index.cmp(&right_index));
                assert_eq!(left.partial_cmp(right), Some(left.cmp(right)));
            }
        }
    }

    #[test]
    fn test_last_known_spec_has_no_successor() {
        assert_eq!(OlSpecId::V1.successor(), None);
    }
}
