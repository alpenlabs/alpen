use std::ops::{Add, Sub};

use crate::Diff;

/// Diffs for large numbers which will increment or decrement by small sizes
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NumDiff<N: Unsigned, Delta: Unsigned + SmallerThan<N>> {
    #[default]
    None,
    Increment(Delta),
    Decrement(Delta),
    Replace(N),
}

impl<N, Delta> Diff for NumDiff<N, Delta>
where
    N: Unsigned + Add<Output = N> + Sub<Output = N> + Copy,
    Delta: Unsigned + SmallerThan<N> + IntoLarger<N> + Copy,
{
    type Target = N;

    fn is_default(&self) -> bool {
        matches!(self, NumDiff::None)
    }

    fn apply(&self, source: &mut Self::Target) -> Result<(), crate::ApplyError> {
        match self {
            NumDiff::None => Ok(()),
            NumDiff::Increment(delta) => {
                *source = *source + (*delta).into_larger();
                Ok(())
            }
            NumDiff::Decrement(delta) => {
                *source = *source - (*delta).into_larger();
                Ok(())
            }
            NumDiff::Replace(value) => {
                *source = *value;
                Ok(())
            }
        }
    }
}

/// Trait that is implemented by all unsigned integers. Maybe make it a sealed trait, or use
/// num-traits crate?
pub trait Unsigned {}
impl Unsigned for u8 {}
impl Unsigned for u16 {}
impl Unsigned for u32 {}
impl Unsigned for u64 {}
impl Unsigned for usize {}
impl Unsigned for u128 {}

/// Trait that is implemented by all unsigned integers that are smaller than the given Type.
pub trait SmallerThan<Other> {}

impl SmallerThan<u128> for u64 {}
impl SmallerThan<u128> for u32 {}
impl SmallerThan<u128> for u16 {}
impl SmallerThan<u128> for u8 {}

impl SmallerThan<u64> for u32 {}
impl SmallerThan<u64> for u16 {}
impl SmallerThan<u64> for u8 {}

impl SmallerThan<u32> for u16 {}
impl SmallerThan<u32> for u8 {}

impl SmallerThan<u16> for u8 {}
// ... other as necessary

/// Trait to convert a smaller unsigned integer into a larger one.
pub trait IntoLarger<N: Unsigned>: Unsigned + SmallerThan<N> {
    fn into_larger(self) -> N;
}

macro_rules! impl_into_larger {
    ($from:ty => [$($to:ty),*]) => {
        $(
            impl IntoLarger<$to> for $from {
                fn into_larger(self) -> $to {
                    self as $to
                }
            }
        )*
    };
}

impl_into_larger!(u8 => [u16, u32, u64, u128]);
impl_into_larger!(u16 => [u32, u64, u128]);
impl_into_larger!(u32 => [u64, u128]);
impl_into_larger!(u64 => [u128]);
