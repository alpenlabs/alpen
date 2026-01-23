//! Common encoding types for DA structures.

use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Byte vector encoded with a big-endian u16 length prefix.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct U16LenBytes {
    /// Raw byte payload.
    inner: Vec<u8>,
}

impl U16LenBytes {
    /// Creates a new [`U16LenBytes`] from a byte vector.
    pub fn new(inner: Vec<u8>) -> Self {
        Self { inner }
    }

    /// Returns a slice of the inner byte vector.
    pub fn as_slice(&self) -> &[u8] {
        &self.inner
    }
}

impl Codec for U16LenBytes {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let len = u16::try_from(self.inner.len()).map_err(|_| CodecError::OverflowContainer)?;
        len.encode(enc)?;
        enc.write_buf(&self.inner)
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = u16::decode(dec)? as usize;
        let mut buf = vec![0u8; len];
        dec.read_buf(&mut buf)?;
        Ok(Self { inner: buf })
    }
}

/// List encoded with a big-endian u16 length prefix.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct U16LenList<T> {
    /// Encoded entries.
    entries: Vec<T>,
}

impl<T> U16LenList<T> {
    /// Creates a new [`U16LenList`] from a vector of entries.
    pub fn new(entries: Vec<T>) -> Self {
        Self { entries }
    }

    /// Returns a slice of the entries.
    pub fn entries(&self) -> &[T] {
        &self.entries
    }

    /// Consumes the list and returns the entries as a vector.
    pub fn into_entries(self) -> Vec<T> {
        self.entries
    }
}

impl<T: Codec> Codec for U16LenList<T> {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let len = u16::try_from(self.entries.len()).map_err(|_| CodecError::OverflowContainer)?;
        len.encode(enc)?;
        for entry in &self.entries {
            entry.encode(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = u16::decode(dec)? as usize;
        let mut entries = Vec::with_capacity(len);
        for _ in 0..len {
            entries.push(T::decode(dec)?);
        }
        Ok(Self { entries })
    }
}
