//! Simple counter type.

use std::{
    marker::PhantomData,
    ops::{Add, AddAssign},
};

use crate::{Codec, CodecError, CodecResult, CompoundMember, DaWrite, Decoder, Encoder};

/// Describes a value that can be updated by some counter.
pub trait CounterScheme {
    /// The base value we're updating.
    type Base;

    /// The increment type.
    type Incr: Clone + Default + Codec;

    /// Returns if the increment is zero.
    fn is_zero(incr: &Self::Incr) -> bool;

    /// Updates the base value by the change.
    fn update(base: &mut Self::Base, incr: &Self::Incr);
}

// This does the addition directly, which may not allow for decrementing.
macro_rules! inst_direct_ctr_schemes {
    ( $( $name:ident ($basety:ident, $incrty:ident); )* ) => {
        $(
            pub struct $name;

            impl CounterScheme for $name {
                type Base = $basety;
                type Incr = $incrty;

                fn is_zero(incr: &Self::Incr) -> bool {
                    *incr == 0
                }

                fn update(base: &mut Self::Base, incr: &Self::Incr) {
                    *base += (*incr as $basety);
                }
            }
        )*
    };
}

// This casts to a more general intermediate type before converting down to the target.
macro_rules! inst_via_ctr_schemes {
    ( $( $name:ident ($basety:ident, $incrty:ident; $viaty:ident); )* ) => {
        $(
            pub struct $name;

            impl CounterScheme for $name {
                type Base = $basety;
                type Incr = $incrty;

                fn is_zero(incr: &Self::Incr) -> bool {
                    *incr == 0
                }

                fn update(base: &mut Self::Base, incr: &Self::Incr) {
                    // TODO add more overflow checks here
                    *base = ((*base as $viaty) + (*incr as $viaty)) as $basety;
                }
            }
        )*
    };
}

inst_direct_ctr_schemes! {
    CtrU64ByU8(u64, u8);
    CtrU64ByU16(u64, u16);
    CtrU32ByU8(u64, u8);
    CtrU32ByU16(u32, u16);
    CtrI64ByI8(i64, i8);
    CtrI64ByI16(i64, i16);
}

inst_via_ctr_schemes! {
    CtrU64ByI8(u64, i8; i64);
    CtrU64ByI16(u64, i16; i64);
    CtrU32ByI8(u32, i8; i64);
    CtrU32ByI16(u32, i16; i64);
    CtrI32ByI8(i32, i8; i64);
    CtrI32ByI16(i32, i16; i64);
}

#[derive(Copy, Clone, Debug, Default)]
pub enum DaCounter<S: CounterScheme> {
    /// Do not change the target.
    #[default]
    Unchanged,

    /// Change the target by T.
    ///
    /// It is malformed for this to be "zero".
    Changed(S::Incr),
}

impl<S: CounterScheme> DaCounter<S> {
    pub fn new_unchanged() -> Self {
        Self::Unchanged
    }

    pub fn is_changed(&self) -> bool {
        matches!(&self, Self::Changed(_))
    }

    /// Returns the value we're changing by, if it's being changed.
    pub fn diff(&self) -> Option<&S::Incr> {
        match self {
            Self::Unchanged => None,
            Self::Changed(v) => Some(v),
        }
    }
}

impl<S: CounterScheme> DaCounter<S> {
    pub fn new_changed(v: S::Incr) -> Self {
        if S::is_zero(&v) {
            Self::new_unchanged()
        } else {
            Self::Changed(v)
        }
    }

    pub fn set_diff(&mut self, d: S::Incr) {
        if S::is_zero(&d) {
            *self = Self::Unchanged;
        } else {
            *self = Self::Changed(d);
        }
    }

    /// If we're changing the value by "zero" then
    pub fn normalize(&mut self) {
        match self {
            Self::Changed(v) => {
                if S::is_zero(v) {
                    *self = Self::Unchanged
                }
            }
            _ => {}
        }
    }
}

impl<S: CounterScheme> DaWrite for DaCounter<S> {
    type Target = S::Base;

    fn is_default(&self) -> bool {
        !self.is_changed()
    }

    fn apply(&self, target: &mut Self::Target) {
        if let Self::Changed(v) = self {
            S::update(target, v);
        }
    }
}

impl<S: CounterScheme> CompoundMember for DaCounter<S> {
    fn default() -> Self {
        Self::new_unchanged()
    }

    fn is_default(&self) -> bool {
        <Self as DaWrite>::is_default(self)
    }

    fn decode_set(dec: &mut impl Decoder) -> CodecResult<Self> {
        Ok(Self::new_changed(<S::Incr as Codec>::decode(dec)?))
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
    use super::{CtrU64ByI16, DaCounter};
    use crate::DaWrite;

    #[test]
    fn test_counter_simple() {
        let ctr1 = DaCounter::<CtrU64ByI16>::new_unchanged();
        let ctr2 = DaCounter::<CtrU64ByI16>::new_changed(1);
        let ctr3 = DaCounter::<CtrU64ByI16>::new_changed(-3);

        let mut v = 32;

        ctr1.apply(&mut v);
        assert_eq!(v, 32);

        ctr2.apply(&mut v);
        assert_eq!(v, 33);

        ctr3.apply(&mut v);
        assert_eq!(v, 30);
    }
}
