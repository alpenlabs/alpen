//! Register DA type.

use crate::{Codec, CodecError, CodecResult, CompoundMember, DaWrite, Decoder, Encoder};

/// A register value.
///
/// This simply wholly replaces the target with a new value if there is one.
#[derive(Clone, Debug)]
pub struct DaRegister<T> {
    new_value: Option<T>,
}

impl<T> DaRegister<T> {
    /// Constructs a new instance with a possible write.
    pub fn new(new_value: Option<T>) -> Self {
        Self { new_value }
    }

    /// Constructs a new instance that sets some value.
    pub fn new_set(v: T) -> Self {
        Self::new(Some(v))
    }

    /// Constructs a new instance that does not write.
    pub fn new_unset() -> Self {
        Self::new(None)
    }

    /// Overwrites value we're setting.
    pub fn set(&mut self, v: T) {
        self.new_value = Some(v);
    }

    /// Gets the new value being written, if present.
    pub fn new_value(&self) -> Option<&T> {
        self.new_value.as_ref()
    }
}

impl<T: Clone + Eq> DaRegister<T> {
    /// Constructs a new instance by comparing an original and new value,
    /// cloning the new one if it's different.
    ///
    /// This only really makes sense for registers since they're the only type
    /// we can consistently do this with.
    pub fn compare(orig: &T, new: &T) -> Self {
        if new == orig {
            Self::new_unset()
        } else {
            Self::new_set(new.clone())
        }
    }
}

impl<T: Codec> DaRegister<T> {
    /// Encodes the inner value, if set.  Returns error if unset as we should
    /// not have reached this point and should assume we're
    /// [`Default::default`].
    pub fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        if let Some(v) = &self.new_value {
            v.encode(enc)
        } else {
            Err(CodecError::MalformedField("tried to encode unset register"))
        }
    }
}

impl<T> Default for DaRegister<T> {
    fn default() -> Self {
        Self { new_value: None }
    }
}

impl<T: Clone> DaRegister<T> {
    /// Applies this register to a target of a different type via [`Into`] conversion.
    ///
    /// This is useful when the register stores a wrapper type (e.g., `CodecU256`)
    /// but the target field is the unwrapped type (e.g., `U256`).
    pub fn apply_into<U>(&self, target: &mut U)
    where
        T: Into<U>,
    {
        if let Some(v) = self.new_value.clone() {
            *target = v.into();
        }
    }
}

impl<T: Clone> DaWrite for DaRegister<T> {
    type Target = T;

    type Context = ();

    type Error = crate::DaError;

    fn is_default(&self) -> bool {
        self.new_value.is_none()
    }

    fn apply(
        &self,
        target: &mut Self::Target,
        _context: &Self::Context,
    ) -> Result<(), Self::Error> {
        if let Some(v) = self.new_value.clone() {
            *target = v;
        }
        Ok(())
    }
}

impl<T: Codec> Codec for DaRegister<T> {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Ok(if bool::decode(dec)? {
            Self::new_set(T::decode(dec)?)
        } else {
            Self::new_unset()
        })
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        match &self.new_value {
            Some(v) => {
                true.encode(enc)?;
                v.encode(enc)?;
            }
            None => {
                false.encode(enc)?;
            }
        }
        Ok(())
    }
}

impl<T: Codec + Clone> CompoundMember for DaRegister<T> {
    fn default() -> Self {
        DaRegister::new_unset()
    }

    fn is_default(&self) -> bool {
        <DaRegister<_> as DaWrite>::is_default(self)
    }

    fn decode_set(dec: &mut impl Decoder) -> CodecResult<Self> {
        let v = T::decode(dec)?;
        Ok(Self::new_set(v))
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        if let Some(v) = &self.new_value {
            v.encode(enc)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::DaRegister;
    use crate::{ContextlessDaWrite, DaWrite, decode_buf_exact, encode_to_vec};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Wrapper(u64);

    impl From<Wrapper> for u128 {
        fn from(value: Wrapper) -> Self {
            value.0.into()
        }
    }

    impl crate::Codec for Wrapper {
        fn encode(&self, enc: &mut impl crate::Encoder) -> Result<(), crate::CodecError> {
            self.0.encode(enc)
        }

        fn decode(dec: &mut impl crate::Decoder) -> Result<Self, crate::CodecError> {
            Ok(Self(u64::decode(dec)?))
        }
    }

    proptest! {
        #[test]
        fn proptest_register_codec_roundtrip(new_value in proptest::option::of(any::<u64>())) {
            let register = DaRegister::new(new_value);

            let encoded = encode_to_vec(&register).expect("test: encode register");
            let decoded: DaRegister<u64> = decode_buf_exact(&encoded).expect("test: decode register");

            prop_assert_eq!(decoded.new_value().copied(), register.new_value().copied());
            prop_assert_eq!(DaWrite::is_default(&decoded), DaWrite::is_default(&register));
        }

        #[test]
        fn proptest_register_compare(orig in any::<u64>(), new in any::<u64>()) {
            let register = DaRegister::compare(&orig, &new);

            if orig == new {
                prop_assert!(DaWrite::is_default(&register));
                prop_assert_eq!(register.new_value(), None);
            } else {
                prop_assert!(!DaWrite::is_default(&register));
                prop_assert_eq!(register.new_value(), Some(&new));
            }
        }

        #[test]
        fn proptest_register_apply(orig in any::<u64>(), new_value in proptest::option::of(any::<u64>())) {
            let register = DaRegister::new(new_value);
            let mut target = orig;

            ContextlessDaWrite::apply(&register, &mut target).expect("test: apply register");

            prop_assert_eq!(target, register.new_value().copied().unwrap_or(orig));
        }

        #[test]
        fn proptest_register_apply_into(target in any::<u128>(), new_value in proptest::option::of(any::<u64>())) {
            let register = DaRegister::new(new_value.map(Wrapper));
            let mut applied = target;

            register.apply_into(&mut applied);

            prop_assert_eq!(
                applied,
                register
                    .new_value()
                    .copied()
                    .map(u128::from)
                    .unwrap_or(target)
            );
        }
    }
}
