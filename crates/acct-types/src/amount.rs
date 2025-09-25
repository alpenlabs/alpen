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
