use crate::impl_transparent_thin_wrapper;

type RawBitcoinAmount = u64;

/// Describes an amount of bitcoin.
///
/// This will eventually be replaced with the more general one, which I am not
/// using here to avoid creating a dependency mess.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct BitcoinAmount(RawBitcoinAmount);

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
