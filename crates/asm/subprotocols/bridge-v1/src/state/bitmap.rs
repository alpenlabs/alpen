//! Operator Bitmap Management
//!
//! This module contains bitmap types and operations for efficiently tracking
//! and filtering operators in various contexts.

use arbitrary::Arbitrary;
use bitvec::prelude::*;
use serde::{Deserialize, Serialize, de::Error as SerdeDeError};
use strata_bridge_types::OperatorIdx;

use crate::{BitmapBytes, BitmapError, OperatorBitmap};

/// Memory-efficient bitmap for tracking active operators in a multisig set.
///
/// This structure provides a compact representation of which operators are active
/// in a specific context (e.g., current multisig, deposit notary set). Uses a
/// dynamic `BitVec` to efficiently handle arbitrary operator index ranges while
/// minimizing memory usage compared to storing operator indices in a `Vec`.
///
/// # Use Cases
///
/// - **Operator Table**: Track which operators are in the current N/N multisig
/// - **Deposit Entries**: Store historical notary operators for each deposit
/// - **Assignment Creation**: Efficiently select operators for new tasks
impl OperatorBitmap {
    pub(crate) fn from_bits(bits: BitVec<u8>) -> Self {
        Self {
            bit_len: bits.len() as u32,
            bytes: BitmapBytes::new(bits.as_raw_slice().to_vec())
                .expect("bridge operator bitmap must stay within SSZ bounds"),
        }
    }

    pub(crate) fn to_bits(&self) -> BitVec<u8> {
        let mut bits = BitVec::from_vec(self.bytes.to_vec());
        bits.truncate(self.bit_len as usize);
        bits
    }

    /// Creates a new empty operator bitmap.
    pub fn new_empty() -> Self {
        Self::from_bits(BitVec::new())
    }

    /// Creates a new operator bitmap with specified size and initial state.
    ///
    /// This is optimized for creating bitmaps with all bits set to the same initial value.
    /// Common use cases include creating cleared bitmaps for tracking previous assignees
    /// or active bitmaps for sequential operators.
    ///
    /// # Parameters
    ///
    /// - `size` - Number of bits in the bitmap
    /// - `initial_state` - Initial state for all bits (true = active, false = inactive)
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Create a bitmap with 5 operators all inactive (for tracking previous assignees)
    /// let cleared = OperatorBitmap::new_with_size(5, false);
    ///
    /// // Create a bitmap with 3 operators all active (for sequential operators 0, 1, 2)
    /// let active = OperatorBitmap::new_with_size(3, true);
    /// ```
    pub fn new_with_size(size: usize, initial_state: bool) -> Self {
        Self::from_bits(BitVec::repeat(initial_state, size))
    }

    /// Returns whether the operator at the given index is active.
    ///
    /// # Parameters
    ///
    /// - `idx` - Operator index to check
    ///
    /// # Returns
    ///
    /// `true` if the operator is active, `false` if not active or index out of bounds
    pub fn is_active(&self, idx: OperatorIdx) -> bool {
        self.to_bits()
            .get(idx as usize)
            .map(|bit| *bit)
            .unwrap_or(false)
    }

    /// Attempts to set the active state of an operator.
    ///
    /// The bitmap maintains sequential indices and only allows extending its size by exactly 1
    /// position at a time. If the index equals the current length, the bitmap is extended by 1.
    /// Indices that would skip positions (greater than current length) are rejected.
    ///
    /// # Parameters
    ///
    /// - `idx` - Operator index to update
    /// - `active` - Whether the operator should be active
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, `Err(BitmapError)` if index would create a gap in the bitmap
    ///
    /// # Index Overflow
    ///
    /// **WARNING**: Since `OperatorIdx` is `u32`, this method cannot handle indices beyond
    /// `u32::MAX` (4,294,967,295). This limits the total number of unique operators that can
    /// ever be registered over the bridge's lifetime.
    pub fn try_set(&mut self, idx: OperatorIdx, active: bool) -> Result<(), BitmapError> {
        let mut bits = self.to_bits();
        let idx_usize = idx as usize;
        // Only allow increasing bitmap size by 1 at a time to maintain sequential indices
        if idx_usize > bits.len() {
            return Err(BitmapError::IndexOutOfBounds {
                index: idx,
                max_valid_index: bits.len() as OperatorIdx,
            });
        }
        if idx_usize == bits.len() {
            bits.resize(idx_usize + 1, false);
        }
        bits.set(idx_usize, active);
        *self = Self::from_bits(bits);
        Ok(())
    }

    /// Returns an iterator over all active operator indices.
    ///
    /// # Index Overflow
    ///
    /// **WARNING**: This method casts internal bit positions (`usize`) to `OperatorIdx` (`u32`).
    /// If the bitmap contains indices beyond `u32::MAX`, this cast will truncate/wrap the values,
    /// producing incorrect results. In practice, this is constrained by the system's operator
    /// registration limit of `u32::MAX` unique operators.
    pub fn active_indices(&self) -> impl Iterator<Item = OperatorIdx> + '_ {
        self.to_bits()
            .iter_ones()
            .map(|index| index as OperatorIdx)
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Returns the number of active operators.
    pub fn active_count(&self) -> usize {
        self.to_bits().count_ones()
    }

    /// Returns the number of inactive operators.
    pub fn inactive_count(&self) -> usize {
        self.len().saturating_sub(self.active_count())
    }

    /// Returns the number of bits in the bitmap.
    pub fn len(&self) -> usize {
        self.bit_len as usize
    }

    /// Returns `true` if the bitmap contains no bits.
    pub fn is_empty(&self) -> bool {
        self.bit_len == 0
    }
}

impl From<BitVec<u8>> for OperatorBitmap {
    fn from(bits: BitVec<u8>) -> Self {
        Self::from_bits(bits)
    }
}

impl Serialize for OperatorBitmap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct BitmapSerde<'a> {
            bit_len: u32,
            bytes: &'a [u8],
        }

        BitmapSerde {
            bit_len: self.bit_len,
            bytes: &self.bytes,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OperatorBitmap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct BitmapSerde {
            bit_len: u32,
            bytes: Vec<u8>,
        }

        let bitmap = BitmapSerde::deserialize(deserializer)?;
        Ok(Self {
            bit_len: bitmap.bit_len,
            bytes: BitmapBytes::new(bitmap.bytes)
                .map_err(|err| SerdeDeError::custom(err.to_string()))?,
        })
    }
}

impl<'a> Arbitrary<'a> for OperatorBitmap {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate a random number of operators between 2 and 20
        let num_operators = u.int_in_range(2..=20)?;

        // Create a random bitmap by generating random bits for each operator
        let mut bits = BitVec::with_capacity(num_operators);
        for _ in 0..num_operators {
            let bit = u.int_in_range(0..=1)? == 1;
            bits.push(bit);
        }
        if bits.not_any() {
            bits.set(0, true);
        }

        Ok(OperatorBitmap::from(bits))
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_test_utils::ArbitraryGenerator;

    use super::*;

    #[test]
    fn test_operator_bitmap_new_empty() {
        let bitmap = OperatorBitmap::new_empty();
        assert!(bitmap.is_empty());
        assert_eq!(bitmap.active_count(), 0);
        assert_eq!(bitmap.active_indices().count(), 0);
    }

    #[test]
    fn test_operator_bitmap_new_with_size() {
        // Test creating cleared bitmap
        let cleared_bitmap = OperatorBitmap::new_with_size(5, false);
        assert!(!cleared_bitmap.is_empty());
        assert_eq!(cleared_bitmap.len(), 5);
        assert_eq!(cleared_bitmap.active_count(), 0);
        assert_eq!(cleared_bitmap.active_indices().count(), 0);

        // Check individual bits are all false
        for i in 0..5 {
            assert!(!cleared_bitmap.is_active(i));
        }
        assert!(!cleared_bitmap.is_active(5)); // Out of bounds should be false

        // Test creating active bitmap
        let active_bitmap = OperatorBitmap::new_with_size(3, true);
        assert!(!active_bitmap.is_empty());
        assert_eq!(active_bitmap.len(), 3);
        assert_eq!(active_bitmap.active_count(), 3);
        assert_eq!(
            active_bitmap.active_indices().collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        // Check individual bits are all true
        for i in 0..3 {
            assert!(active_bitmap.is_active(i));
        }
        assert!(!active_bitmap.is_active(3)); // Out of bounds should be false
    }

    #[test]
    fn test_operator_bitmap_try_set() {
        let mut bitmap = OperatorBitmap::new_empty();

        // Setting bit 0 should work
        assert!(bitmap.try_set(0, true).is_ok());
        assert!(bitmap.is_active(0));
        assert_eq!(bitmap.active_count(), 1);

        // Setting bit 1 should work (sequential)
        assert!(bitmap.try_set(1, true).is_ok());
        assert!(bitmap.is_active(1));
        assert_eq!(bitmap.active_count(), 2);

        // Setting bit 0 to false should work
        assert!(bitmap.try_set(0, false).is_ok());
        assert!(!bitmap.is_active(0));
        assert_eq!(bitmap.active_count(), 1);

        // Trying to set bit 3 (skipping 2) should fail
        assert_eq!(
            bitmap.try_set(3, true),
            Err(BitmapError::IndexOutOfBounds {
                index: 3,
                max_valid_index: 2
            })
        );
        assert_eq!(bitmap.active_count(), 1);

        // Use a large initial bitmap
        let mut bitmap = OperatorBitmap::new_with_size(500, true);

        // Setting bit active doesn't change the active count
        assert!(bitmap.try_set(0, true).is_ok());
        assert_eq!(bitmap.active_count(), 500);

        // Setting bit inactive changes change the active count
        assert!(bitmap.try_set(0, false).is_ok());
        assert_eq!(bitmap.active_count(), 499);

        // Setting bit 500 should work (sequential)
        assert!(bitmap.try_set(500, true).is_ok());
        assert!(bitmap.is_active(500));
        assert_eq!(bitmap.active_count(), 500);

        // Trying to unset bit 1000 (skipping 501..) should fail
        assert_eq!(
            bitmap.try_set(1000, false),
            Err(BitmapError::IndexOutOfBounds {
                index: 1000,
                max_valid_index: 501
            })
        );
        assert_eq!(bitmap.active_count(), 500);
    }

    #[test]
    fn test_operator_bitmap_serialization_roundtrip() {
        let mut arb = ArbitraryGenerator::new();
        let bitmap: OperatorBitmap = arb.generate();
        let serialized_bytes = bitmap.as_ssz_bytes();
        let deserialized_bitmap = OperatorBitmap::from_ssz_bytes(&serialized_bytes).unwrap();
        assert_eq!(bitmap, deserialized_bitmap);
    }
}
