//! Simple counter type.

use std::{
    marker::PhantomData,
    ops::{Add, AddAssign},
};

use crate::{Codec, CodecError, CodecResult, CompoundMember, DaWrite, Decoder, Encoder};

#[derive(Copy, Clone, Debug, Default)]
pub enum DaCounter<T> {
    /// Do not change the target.
    #[default]
    Unchanged,

    /// Change the target by T.
    ///
    /// It is malformed for this to be "zero".
    Changed(T),
}

impl<T> DaCounter<T> {
    pub fn new_unchanged() -> Self {
        Self::Unchanged
    }

    pub fn is_changed(&self) -> bool {
        matches!(&self, Self::Changed(_))
    }

    /// Returns the value we're changing by, if it's being changed.
    pub fn diff(&self) -> Option<&T> {
        match self {
            Self::Unchanged => None,
            Self::Changed(v) => Some(v),
        }
    }
}

impl<T: Eq + Default> DaCounter<T> {
    pub fn new_changed(v: T) -> Self {
        if v == T::default() {
            Self::new_unchanged()
        } else {
            Self::Changed(v)
        }
    }

    pub fn set_diff(&mut self, d: T) {
        if d == T::default() {
            *self = Self::Unchanged;
        } else {
            *self = Self::Changed(d);
        }
    }

    /// If we're changing the value by "zero" then
    pub fn normalize(&mut self) {
        match self {
            Self::Changed(v) => {
                if *v == T::default() {
                    *self = Self::Unchanged
                }
            }
            _ => {}
        }
    }
}

impl<T: Copy + Eq + Default + Add<Output = T>> DaCounter<T> {
    pub fn add_diff(&mut self, d: T) {
        let v = self.diff().copied().unwrap_or_default();
        self.set_diff(v + d);
    }
}

impl<T> DaWrite for DaCounter<T>
where
    T: Copy + Default + AddAssign<T>,
{
    type Target = T;

    fn is_default(&self) -> bool {
        matches!(&self, Self::Unchanged)
    }

    fn apply(&self, target: &mut Self::Target) {
        if let Self::Changed(d) = &self {
            *target += *d;
        }
    }
}

impl<T: Copy + Eq + Default + AddAssign<T> + Codec> CompoundMember for DaCounter<T> {
    fn default() -> Self {
        Self::new_unchanged()
    }

    fn is_default(&self) -> bool {
        <Self as DaWrite>::is_default(self)
    }

    fn decode_set(dec: &mut impl Decoder) -> CodecResult<Self> {
        Ok(Self::new_changed(T::decode(dec)?))
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        if <Self as CompoundMember>::is_default(self) {
            return Err(CodecError::WriteUnnecessaryDefault);
        }

        if let DaCounter::Changed(d) = &self {
            d.encode(enc)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DaCounter;
    use crate::DaWrite;

    #[test]
    fn test_counter_simple() {
        let ctr1 = DaCounter::<i16>::new_unchanged();
        let ctr2 = DaCounter::<i16>::new_changed(1);
        let ctr3 = DaCounter::<i16>::new_changed(-3);

        let mut v = 32;

        ctr1.apply(&mut v);
        assert_eq!(v, 32);

        ctr2.apply(&mut v);
        assert_eq!(v, 33);

        ctr3.apply(&mut v);
        assert_eq!(v, 30);
    }
}
