use ssz_derive::{Decode, Encode};
use tree_hash::TreeHash;

use crate::impl_transparent_thin_wrapper;

type RawBitcoinAmount = u64;

/// Describes an amount of bitcoin.
///
/// This will eventually be replaced with the more general one, which I am not
/// using here to avoid creating a dependency mess.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct BitcoinAmount(RawBitcoinAmount);

// Manual TreeHash implementation for transparent wrapper
impl TreeHash for BitcoinAmount {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <u64 as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <u64 as TreeHash>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <u64 as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <u64 as TreeHash>::tree_hash_root(&self.0)
    }
}

impl_transparent_thin_wrapper!(BitcoinAmount => RawBitcoinAmount);

impl BitcoinAmount {
    pub fn zero() -> Self {
        Self(0)
    }

    /// Sums an iterator of multiple amounts, panicking on overflow.
    pub fn sum(iter: impl IntoIterator<Item = BitcoinAmount>) -> BitcoinAmount {
        let v = iter.into_iter().fold(0u64, |a, e| {
            a.checked_add(*e).expect("acctsys: amount overflow")
        });

        Self(v)
    }

    /// Returns if the amount is zero.
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    use super::*;

    #[test]
    fn test_bitcoin_amount_ssz_roundtrip() {
        let amount = BitcoinAmount::new(12345);
        let encoded = amount.as_ssz_bytes();
        let decoded = BitcoinAmount::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(amount, decoded);
    }

    #[test]
    fn test_bitcoin_amount_tree_hash() {
        let amount = BitcoinAmount::new(1000);
        let hash = amount.tree_hash_root();
        // Should produce same hash as underlying u64
        assert_eq!(hash, <u64 as TreeHash>::tree_hash_root(&1000u64));
    }

    #[test]
    fn test_bitcoin_amount_zero_ssz() {
        let zero = BitcoinAmount::zero();
        let encoded = zero.as_ssz_bytes();
        let decoded = BitcoinAmount::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(zero, decoded);
        assert!(decoded.is_zero());
    }
}
