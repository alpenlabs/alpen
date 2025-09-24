use crate::impl_thin_wrapper;

/// Describes an amount of bitcoin.
///
/// This will eventually be replaced with the more general one, which I am not
/// using here to avoid creating a dependency mess.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct BitcoinAmount(u64);

impl_thin_wrapper!(BitcoinAmount => u64);
