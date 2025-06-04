//! Register DA type.

use crate::{BuilderError, Codec, CodecError, CodecResult, DaBuilder, DaWrite, Decoder, Encoder};

/// A register value.
///
/// This simply wholly replaces the target with a new value if there is one.
#[derive(Clone, Debug)]
pub struct DaRegister<T> {
    new_value: Option<T>,
}

impl<T> DaRegister<T> {
    pub fn new(new_value: Option<T>) -> Self {
        Self { new_value }
    }

    pub fn new_set(v: T) -> Self {
        Self::new(Some(v))
    }

    pub fn new_unset() -> Self {
        Self::new(None)
    }

    /// Overwrites value we're setting.
    pub fn set(&mut self, v: T) {
        self.new_value = Some(v);
    }

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
    /// Constructs a `Some` instance from a decoder.
    pub fn set_from_decoder(dec: &mut impl Decoder) -> CodecResult<Self> {
        let v = T::decode(dec)?;
        Ok(Self::new_set(v))
    }

    /// Encodes the inner value, if set.  Returns error if unset as we should
    /// not have reached this point and should assume we're
    /// [`Default::default`].
    pub fn encode_set(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        if let Some(v) = &self.new_value {
            v.encode(enc)
        } else {
            Err(CodecError::WriteUnnecessaryDefault)
        }
    }

    /// Encodes the inner value, if set.  Does nothing if unset.
    ///
    /// MUST be used in the context of a compound which can do bit flagging
    /// to properly track set/unset fields.
    pub fn encode_if_set(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        if let Some(v) = &self.new_value {
            v.encode(enc)?;
        }
        Ok(())
    }
}

impl<T> Default for DaRegister<T> {
    fn default() -> Self {
        Self { new_value: None }
    }
}

impl<T: Clone> DaWrite for DaRegister<T> {
    type Target = T;

    fn is_default(&self) -> bool {
        self.new_value.is_none()
    }

    fn apply(&self, target: &mut Self::Target) {
        if let Some(v) = self.new_value.clone() {
            *target = v;
        }
    }
}

/// Builder for [`DaRegister`].
pub struct DaRegisterBuilder<T> {
    original: T,
    new: T,
}

impl<T> DaRegisterBuilder<T> {
    pub fn value(&self) -> &T {
        &self.new
    }

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
