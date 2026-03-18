use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};

/// The ID of an operator.
///
/// We define it as a type alias over [`u32`] instead of a newtype because we perform a bunch of
/// mathematical operations on it while managing the operator table.
pub type OperatorIdx = u32;

/// Sentinel value representing "no specific operator selected."
const NO_SELECTION_SENTINEL: u32 = u32::MAX;

/// Encapsulates the user's operator selection for a withdrawal assignment.
///
/// Wraps a [`u32`] where [`u32::MAX`] means "any operator" (random assignment)
/// and any other value is a specific [`OperatorIdx`].
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Arbitrary,
    Encode,
    Decode,
)]
pub struct OperatorSelection(u32);

impl OperatorSelection {
    /// Creates a selection meaning "assign to any eligible operator."
    pub fn any() -> Self {
        Self(NO_SELECTION_SENTINEL)
    }

    /// Creates a selection for a specific operator index.
    ///
    /// # Panics
    ///
    /// Panics if `idx` equals [`u32::MAX`], which is reserved as the "any" sentinel.
    pub fn specific(idx: OperatorIdx) -> Self {
        assert_ne!(
            idx, NO_SELECTION_SENTINEL,
            "u32::MAX is reserved for the 'any' sentinel"
        );
        Self(idx)
    }

    /// Returns the specific operator index, or [`None`] if this is an "any" selection.
    pub fn as_specific(&self) -> Option<OperatorIdx> {
        (self.0 != NO_SELECTION_SENTINEL).then_some(self.0)
    }

    /// Returns the raw [`u32`] representation.
    pub fn raw(self) -> u32 {
        self.0
    }

    /// Constructs from a raw [`u32`], as decoded from the wire.
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}
