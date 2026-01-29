//! Simple counter type.

use crate::{
    BuilderError, Codec, CodecError, CodecResult, CompoundMember, DaBuilder, DaWrite, Decoder,
    Encoder, Varint,
};

/// Describes scheme for a counter value and the quantity that it can change by.
pub trait CounterScheme {
    /// The base value we're updating.
    type Base;

    /// The increment type.
    type Incr: Clone + Default + Codec;

    /// Returns if the increment is zero.
    fn is_zero(incr: &Self::Incr) -> bool;

    /// Updates the base value by the change.
    fn update(base: &mut Self::Base, incr: &Self::Incr);

    /// Compares two base values and returns the diff from `a` to `b`, in terms
    /// of an increment.
    ///
    /// Returns `None` if invalid or out of range.
    // TODO should these be passed by ref?
    fn compare(a: Self::Base, b: Self::Base) -> Option<Self::Incr>;
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
        if let Self::Changed(v) = self
            && S::is_zero(v)
        {
            *self = Self::Unchanged
        }
    }
}

impl<S: CounterScheme> DaWrite for DaCounter<S> {
    type Target = S::Base;

    type Context = ();

    fn is_default(&self) -> bool {
        !self.is_changed()
    }

    fn apply(
        &self,
        target: &mut Self::Target,
        _context: &Self::Context,
    ) -> Result<(), crate::DaError> {
        if let Self::Changed(v) = self {
            S::update(target, v);
        }
        Ok(())
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
            return Err(CodecError::InvalidVariant("counter"));
        }

        if let DaCounter::Changed(d) = &self {
            d.encode(enc)
        } else {
            Ok(())
        }
    }
}

/// Builder for [`DaCounter`].
pub struct DaCounterBuilder<S: CounterScheme> {
    original: S::Base,
    new: S::Base,
}

impl<S: CounterScheme> DaCounterBuilder<S>
where
    S::Base: Clone,
    S::Incr: Clone,
{
    /// Returns the new value currently being tracked.
    pub fn new_value(&self) -> &S::Base {
        &self.new
    }

    /// Updates the value, ensuring the diff is in-bounds.
    pub fn set(&mut self, v: S::Base) -> Result<(), BuilderError> {
        S::compare(self.original.clone(), v.clone()).ok_or(BuilderError::OutOfBoundsValue)?;
        self.new = v;
        Ok(())
    }

    /// Updates the value by adding an increment to it, ensuring it's in-bounds.
    pub fn add(&mut self, d: &S::Incr) -> Result<(), BuilderError> {
        let mut nv = self.new.clone();
        S::update(&mut nv, d);
        self.set(nv)
    }

    /// Sets the value without checking if the diff will be in-bounds.  This may
    /// trigger an error when building the final write if out of bounds by then.
    pub fn set_unchecked(&mut self, v: S::Base) {
        self.new = v;
    }

    fn compute_incr(&self) -> Option<S::Incr> {
        S::compare(self.original.clone(), self.new.clone())
    }
}

impl<S: CounterScheme> DaBuilder<S::Base> for DaCounterBuilder<S>
where
    S::Base: Clone,
{
    type Write = DaCounter<S>;

    fn from_source(t: S::Base) -> Self {
        Self {
            original: t.clone(),
            new: t,
        }
    }

    fn into_write(self) -> Result<Self::Write, BuilderError> {
        let d = self.compute_incr().ok_or(BuilderError::OutOfBoundsValue)?;
        Ok(if S::is_zero(&d) {
            DaCounter::new_unchanged()
        } else {
            DaCounter::new_changed(d)
        })
    }
}

// This does the addition directly, which may not allow for decrementing.
macro_rules! inst_direct_ctr_schemes {
    ( $( $name:ident ($basety:ident, $incrty:ident); )* ) => {
        $(
            #[derive(Copy, Clone, Debug, Default)]
            pub struct $name;

            impl $crate::CounterScheme for $name {
                type Base = $basety;
                type Incr = $incrty;

                fn is_zero(incr: &Self::Incr) -> bool {
                    *incr == 0
                }

                fn update(base: &mut Self::Base, incr: &Self::Incr) {
                    *base += (*incr as $basety);
                }

                fn compare(a: Self::Base, b: Self::Base) -> Option<Self::Incr> {
                    <$incrty>::try_from(<$basety>::checked_sub(b, a)?).ok()
                }
            }
        )*
    };
}

// This casts to a more general intermediate type before converting down to the target.
macro_rules! inst_via_ctr_schemes {
    ( $( $name:ident ($basety:ident, $incrty:ident; $viaty:ident); )* ) => {
        $(
            #[derive(Copy, Clone, Debug, Default)]
            pub struct $name;

            impl $crate::CounterScheme for $name {
                type Base = $basety;
                type Incr = $incrty;

                fn is_zero(incr: &Self::Incr) -> bool {
                    *incr == 0
                }

                fn update(base: &mut Self::Base, incr: &Self::Incr) {
                    // TODO add more overflow checks here
                    *base = ((*base as $viaty) + (*incr as $viaty)) as $basety;
                }

                fn compare(a: Self::Base, b: Self::Base) -> Option<Self::Incr> {
                    let aa = <$viaty>::try_from(a).ok()?;
                    let bb = <$viaty>::try_from(b).ok()?;
                    <$incrty>::try_from(<$viaty>::checked_sub(bb, aa)?).ok()
                }
            }
        )*
    };
}

/// Counter schemes.
pub mod counter_schemes {
    use super::Varint;
    use crate::{Codec, CodecError, Decoder, Encoder};

    inst_direct_ctr_schemes! {
        CtrU64ByU8(u64, u8);
        CtrU64ByU16(u64, u16);
        CtrU32ByU8(u32, u8);
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

    // ==================== Varint-based counter schemes ====================
    //
    // TODO: ideally strata-common defines Varint and SignedVarint (with #derive(Default)),
    // so that newtype wrapper is not required and varints can be used as an `Incr` type directly.

    /// Unsigned varint increment.
    ///
    /// Wraps [`Varint`] for use as a counter increment type. Encoding sizes:
    /// - 0 to 127: 1 byte
    /// - 128 to 16383: 2 bytes
    /// - 16384 to ~1 billion: 4 bytes
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct UnsignedVarintIncr(Varint);

    impl Default for UnsignedVarintIncr {
        fn default() -> Self {
            Self(Varint::new(0).unwrap())
        }
    }

    impl UnsignedVarintIncr {
        /// Maximum representable value (~1 billion).
        pub const MAX: u32 = strata_codec::VARINT_MAX;

        /// Creates a new unsigned varint increment.
        ///
        /// Returns `None` if the value exceeds [`Self::MAX`].
        pub fn new(v: u32) -> Option<Self> {
            Varint::new(v).map(Self)
        }

        /// Returns the inner u32 value.
        pub fn inner(self) -> u32 {
            self.0.inner()
        }
    }

    impl Codec for UnsignedVarintIncr {
        fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
            self.0.encode(enc)
        }

        fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
            Ok(Self(Varint::decode(dec)?))
        }
    }

    /// Counter scheme for u64 base with unsigned varint increment.
    ///
    /// Encoding sizes:
    /// - 0 to 127: 1 byte
    /// - 128 to 16383: 2 bytes
    /// - 16384 to ~1 billion: 4 bytes
    ///
    /// Use for monotonically increasing counters (e.g., nonces).
    #[derive(Copy, Clone, Debug, Default)]
    pub struct CtrU64ByUnsignedVarint;

    impl crate::CounterScheme for CtrU64ByUnsignedVarint {
        type Base = u64;
        type Incr = UnsignedVarintIncr;

        fn is_zero(incr: &Self::Incr) -> bool {
            incr.inner() == 0
        }

        fn update(base: &mut Self::Base, incr: &Self::Incr) {
            *base = base.saturating_add(incr.inner() as u64);
        }

        fn compare(a: Self::Base, b: Self::Base) -> Option<Self::Incr> {
            let diff = b.checked_sub(a)?;
            let diff_u32 = u32::try_from(diff).ok()?;
            UnsignedVarintIncr::new(diff_u32)
        }
    }

    // ------------------------- ZigZag encoding -------------------------

    /// ZigZag-encodes an i32 into a u32.
    ///
    /// Maps: 0 → 0, -1 → 1, 1 → 2, -2 → 3, 2 → 4, ...
    #[inline]
    pub(crate) const fn zigzag_encode(n: i32) -> u32 {
        ((n << 1) ^ (n >> 31)) as u32
    }

    /// ZigZag-decodes a u32 back into an i32.
    #[inline]
    pub(crate) const fn zigzag_decode(n: u32) -> i32 {
        ((n >> 1) as i32) ^ -((n & 1) as i32)
    }

    /// Signed varint increment using ZigZag encoding.
    ///
    /// Wraps [`Varint`] with ZigZag encoding to support negative values.
    /// Encoding sizes:
    /// - -64 to 63: 1 byte
    /// - -8192 to 8191: 2 bytes
    /// - beyond: 4 bytes
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct SignedVarintIncr(Varint);

    impl Default for SignedVarintIncr {
        fn default() -> Self {
            Self(Varint::new(0).unwrap())
        }
    }

    impl SignedVarintIncr {
        /// Maximum representable value (~536 million).
        pub const MAX: i32 = (strata_codec::VARINT_MAX / 2) as i32;

        /// Minimum representable value (~-536 million).
        pub const MIN: i32 = -((strata_codec::VARINT_MAX / 2) as i32) - 1;

        /// Creates a new signed varint increment.
        ///
        /// Returns `None` if the value is outside [`Self::MIN`]..=[`Self::MAX`].
        pub fn new(v: i32) -> Option<Self> {
            let encoded = zigzag_encode(v);
            Varint::new(encoded).map(Self)
        }

        /// Returns the inner i32 value.
        pub fn inner(self) -> i32 {
            zigzag_decode(self.0.inner())
        }
    }

    impl Codec for SignedVarintIncr {
        fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
            self.0.encode(enc)
        }

        fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
            Ok(Self(Varint::decode(dec)?))
        }
    }

    /// Counter scheme for u64 base with signed varint increment.
    ///
    /// Encoding sizes:
    /// - -64 to 63: 1 byte
    /// - -8192 to 8191: 2 bytes
    /// - beyond: 4 bytes
    ///
    /// Use when counter values can decrease (e.g., balance adjustments).
    #[derive(Copy, Clone, Debug, Default)]
    pub struct CtrU64BySignedVarint;

    impl crate::CounterScheme for CtrU64BySignedVarint {
        type Base = u64;
        type Incr = SignedVarintIncr;

        fn is_zero(incr: &Self::Incr) -> bool {
            incr.inner() == 0
        }

        fn update(base: &mut Self::Base, incr: &Self::Incr) {
            let delta = incr.inner();
            if delta >= 0 {
                *base = base.saturating_add(delta as u64);
            } else {
                *base = base.saturating_sub(delta.unsigned_abs() as u64);
            }
        }

        fn compare(a: Self::Base, b: Self::Base) -> Option<Self::Incr> {
            let diff = (b as i128) - (a as i128);
            let diff_i32 = i32::try_from(diff).ok()?;
            SignedVarintIncr::new(diff_i32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DaCounter,
        counter_schemes::{
            CtrU64ByI16, CtrU64BySignedVarint, CtrU64ByUnsignedVarint, SignedVarintIncr,
            UnsignedVarintIncr,
        },
    };
    use crate::{ContextlessDaWrite, decode_buf_exact, encode_to_vec};

    #[test]
    fn test_counter_simple() {
        let ctr1 = DaCounter::<CtrU64ByI16>::new_unchanged();
        let ctr2 = DaCounter::<CtrU64ByI16>::new_changed(1);
        let ctr3 = DaCounter::<CtrU64ByI16>::new_changed(-3);

        let mut v = 32;

        ctr1.apply(&mut v).unwrap();
        assert_eq!(v, 32);

        ctr2.apply(&mut v).unwrap();
        assert_eq!(v, 33);

        ctr3.apply(&mut v).unwrap();
        assert_eq!(v, 30);
    }

    #[test]
    fn test_varint_incr_encoding_sizes() {
        // Small values (0-127) should use 1 byte
        let small = UnsignedVarintIncr::new(42).unwrap();
        let encoded_small = encode_to_vec(&small).unwrap();
        assert_eq!(encoded_small.len(), 1);

        // Values 128-16383 should use 2 bytes
        let medium = UnsignedVarintIncr::new(1000).unwrap();
        let encoded_medium = encode_to_vec(&medium).unwrap();
        assert_eq!(encoded_medium.len(), 2);

        // Values > 16383 should use 4 bytes
        let large = UnsignedVarintIncr::new(100_000).unwrap();
        let encoded_large = encode_to_vec(&large).unwrap();
        assert_eq!(encoded_large.len(), 4);
    }

    #[test]
    fn test_varint_incr_roundtrip() {
        for val in [0, 1, 127, 128, 1000, 16383, 16384, 100_000, 1_000_000_000] {
            let incr = UnsignedVarintIncr::new(val).unwrap();
            let encoded = encode_to_vec(&incr).unwrap();
            let decoded: UnsignedVarintIncr = decode_buf_exact(&encoded).unwrap();
            assert_eq!(decoded.inner(), val);
        }
    }

    #[test]
    fn test_varint_counter_apply() {
        let incr = UnsignedVarintIncr::new(42).unwrap();
        let ctr = DaCounter::<CtrU64ByUnsignedVarint>::new_changed(incr);

        let mut v = 100u64;
        ctr.apply(&mut v).unwrap();
        assert_eq!(v, 142);
    }

    #[test]
    fn test_varint_counter_large_increment() {
        // Test with a value that would overflow u8 (>255)
        let incr = UnsignedVarintIncr::new(1000).unwrap();
        let ctr = DaCounter::<CtrU64ByUnsignedVarint>::new_changed(incr);

        let mut v = 5000u64;
        ctr.apply(&mut v).unwrap();
        assert_eq!(v, 6000);
    }

    #[test]
    fn test_varint_counter_unchanged() {
        let ctr = DaCounter::<CtrU64ByUnsignedVarint>::new_unchanged();
        assert!(!ctr.is_changed());

        let mut v = 100u64;
        ctr.apply(&mut v).unwrap();
        assert_eq!(v, 100); // Unchanged
    }

    #[test]
    fn test_zigzag_encoding() {
        use super::counter_schemes::{zigzag_decode, zigzag_encode};

        // Verify zigzag mapping: 0 → 0, -1 → 1, 1 → 2, -2 → 3, 2 → 4, etc.
        assert_eq!(zigzag_encode(0), 0);
        assert_eq!(zigzag_encode(-1), 1);
        assert_eq!(zigzag_encode(1), 2);
        assert_eq!(zigzag_encode(-2), 3);
        assert_eq!(zigzag_encode(2), 4);

        // Roundtrip
        for v in [
            0,
            1,
            -1,
            100,
            -100,
            10000,
            -10000,
            i32::MAX / 2,
            i32::MIN / 2,
        ] {
            assert_eq!(zigzag_decode(zigzag_encode(v)), v);
        }
    }

    #[test]
    fn test_signed_varint_incr_encoding_sizes() {
        // Small absolute values (-64 to 63) should use 1 byte
        // zigzag(63) = 126, zigzag(-64) = 127
        let small_pos = SignedVarintIncr::new(63).unwrap();
        let small_neg = SignedVarintIncr::new(-64).unwrap();
        assert_eq!(encode_to_vec(&small_pos).unwrap().len(), 1);
        assert_eq!(encode_to_vec(&small_neg).unwrap().len(), 1);

        // Medium values should use 2 bytes
        let medium_pos = SignedVarintIncr::new(1000).unwrap();
        let medium_neg = SignedVarintIncr::new(-1000).unwrap();
        assert_eq!(encode_to_vec(&medium_pos).unwrap().len(), 2);
        assert_eq!(encode_to_vec(&medium_neg).unwrap().len(), 2);

        // Large values should use 4 bytes
        let large_pos = SignedVarintIncr::new(100_000).unwrap();
        let large_neg = SignedVarintIncr::new(-100_000).unwrap();
        assert_eq!(encode_to_vec(&large_pos).unwrap().len(), 4);
        assert_eq!(encode_to_vec(&large_neg).unwrap().len(), 4);
    }

    #[test]
    fn test_signed_varint_incr_roundtrip() {
        for val in [0, 1, -1, 63, -64, 64, -65, 1000, -1000, 100_000, -100_000] {
            let incr = SignedVarintIncr::new(val).unwrap();
            let encoded = encode_to_vec(&incr).unwrap();
            let decoded: SignedVarintIncr = decode_buf_exact(&encoded).unwrap();
            assert_eq!(decoded.inner(), val);
        }
    }

    #[test]
    fn test_signed_varint_incr_bounds() {
        // Should succeed at bounds
        assert!(SignedVarintIncr::new(SignedVarintIncr::MAX).is_some());
        assert!(SignedVarintIncr::new(SignedVarintIncr::MIN).is_some());

        // Should fail beyond bounds (if representable in i32)
        // MAX + 1 might overflow i32, so we check carefully
        if let Some(beyond_max) = SignedVarintIncr::MAX.checked_add(1) {
            assert!(SignedVarintIncr::new(beyond_max).is_none());
        }
        if let Some(beyond_min) = SignedVarintIncr::MIN.checked_sub(1) {
            assert!(SignedVarintIncr::new(beyond_min).is_none());
        }
    }

    #[test]
    fn test_signed_varint_counter_positive() {
        let incr = SignedVarintIncr::new(42).unwrap();
        let ctr = DaCounter::<CtrU64BySignedVarint>::new_changed(incr);

        let mut v = 100u64;
        ctr.apply(&mut v).unwrap();
        assert_eq!(v, 142);
    }

    #[test]
    fn test_signed_varint_counter_negative() {
        let incr = SignedVarintIncr::new(-30).unwrap();
        let ctr = DaCounter::<CtrU64BySignedVarint>::new_changed(incr);

        let mut v = 100u64;
        ctr.apply(&mut v).unwrap();
        assert_eq!(v, 70);
    }

    #[test]
    fn test_signed_varint_counter_saturation() {
        // Negative increment shouldn't underflow
        let incr = SignedVarintIncr::new(-100).unwrap();
        let ctr = DaCounter::<CtrU64BySignedVarint>::new_changed(incr);

        let mut v = 50u64;
        ctr.apply(&mut v).unwrap();
        assert_eq!(v, 0); // Saturates at 0
    }
}
