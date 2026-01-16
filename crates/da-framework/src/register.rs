//! Register DA type.

use crate::{
    BuilderError, Codec, CodecError, CodecResult, CompoundMember, DaBuilder, DaWrite, Decoder,
    Encoder,
};

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

    fn is_default(&self) -> bool {
        self.new_value.is_none()
    }

    fn apply(
        &self,
        target: &mut Self::Target,
        _context: &Self::Context,
    ) -> Result<(), crate::DaError> {
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

/// Builder for [`DaRegister`].
pub struct DaRegisterBuilder<T> {
    original: T,
    new: T,
}

impl<T> DaRegisterBuilder<T> {
    /// Gets the current value.
    pub fn value(&self) -> &T {
        &self.new
    }

    /// Sets the value.
    pub fn set(&mut self, t: T) {
        self.new = t;
    }
}

impl<T: Clone + Eq> DaBuilder<T> for DaRegisterBuilder<T> {
    type Write = DaRegister<T>;

    fn from_source(t: T) -> Self {
        Self {
            original: t.clone(),
            new: t,
        }
    }

    fn into_write(self) -> Result<Self::Write, BuilderError> {
        if self.new == self.original {
            Ok(DaRegister::new_unset())
        } else {
            Ok(DaRegister::new_set(self.new))
        }
    }
}
