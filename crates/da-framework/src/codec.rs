//! Codec traits.
//!
//! This is actually similar in structure to borsh, but is more streamlines and
//! we can "own" it all and can make some optimizations because we fully
//! understand how it's going to be used.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("oversized container")]
    OversizeContainer,

    #[error("tried to write a default DA value in a context where it is redundant")]
    WriteUnnecessaryDefault,

    #[error("end of input")]
    Eof,
}

pub type CodecResult<T> = Result<T, CodecError>;

/// Container for reading chunks of bytes from a fixed buffer.
pub struct BufCursor {
    buf: Vec<u8>,
    pos: usize,
}

impl BufCursor {
    pub fn new(buf: Vec<u8>) -> Self {
        Self { buf, pos: 0 }
    }

    /// Returns the current position of the buffer cursor.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Returns the remaining bytes in the buffer.
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Returns a slice into the remaining bytes.
    fn rem_slice(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    /// Returns a slice from the remaining bytes.
    ///
    /// Returns error if we try to read past the end.
    pub fn slice_len(&self, len: usize) -> CodecResult<&[u8]> {
        if len > self.remaining() {
            return Err(CodecError::Eof);
        }

        Ok(&self.rem_slice()[..len])
    }

    /// Advances the cursor.
    ///
    /// Returns error if we try to advance past the end.
    pub fn advance(&mut self, len: usize) -> CodecResult<()> {
        let new_pos = self.pos + len;
        if new_pos > self.buf.len() {
            return Err(CodecError::Eof);
        }
        self.pos = new_pos;
        Ok(())
    }
}

/// Decoder trait for consuming bytes from a source buffer
pub trait Decoder {
    /// Tries to read a full constant-size array.
    ///
    /// Returns EOF if unable to satisfy the request.
    fn read_array<const N: usize>(&mut self) -> CodecResult<[u8; N]>;

    /// Tries to read a full buffer.  It's assumed that the target buffer is
    /// sized appropriately for the use case.  If successful, always have filled
    /// the whole target buffer
    ///
    /// Returns EOF if unable to satisfy the request and the contents of the
    /// target buffer are unspecified.
    fn read_buf(&mut self, target: &mut [u8]) -> CodecResult<()>;
}

impl Decoder for BufCursor {
    fn read_array<const N: usize>(&mut self) -> CodecResult<[u8; N]> {
        let mut arr = [0; N];
        let source = self.slice_len(N)?;
        arr.copy_from_slice(source);
        self.advance(N)?;
        Ok(arr)
    }

    fn read_buf(&mut self, target: &mut [u8]) -> CodecResult<()> {
        let source = self.slice_len(target.len())?;
        target.copy_from_slice(source);
        self.advance(target.len())?;
        Ok(())
    }
}

/// Encoder trait for writing bytes into a buffer.
pub trait Encoder {
    /// Writes a raw buf into the buffer.
    ///
    /// Does not prefix a length tag, the caller must do this.
    fn write_buf(&mut self, buf: &[u8]) -> CodecResult<()>;
}

impl Encoder for Vec<u8> {
    fn write_buf(&mut self, buf: &[u8]) -> CodecResult<()> {
        self.extend_from_slice(buf);
        Ok(())
    }
}

/// Describes encoding/decoding for DA types.
pub trait Codec: Sized {
    /// Decodes an instance from a decoder.
    fn decode(dec: &mut impl Decoder) -> CodecResult<Self>;

    /// Encodes an instance into an encoder.
    fn encode(&self, enc: &mut impl Encoder) -> CodecResult<()>;
}

impl<const N: usize> Codec for [u8; N] {
    fn decode(dec: &mut impl Decoder) -> CodecResult<Self> {
        dec.read_array()
    }

    fn encode(&self, enc: &mut impl Encoder) -> CodecResult<()> {
        enc.write_buf(self)
    }
}

macro_rules! int_codec {
    ($ity:ident) => {
        impl Codec for $ity {
            fn decode(dec: &mut impl Decoder) -> CodecResult<Self> {
                const BYTES: usize = ($ity::BITS / 8) as usize;
                let buf = dec.read_array::<BYTES>()?;
                Ok(<$ity>::from_be_bytes(buf))
            }

            fn encode(&self, enc: &mut impl Encoder) -> CodecResult<()> {
                enc.write_buf(&self.to_be_bytes()[..])
            }
        }
    };
}

int_codec!(u8);
int_codec!(u16);
int_codec!(u32);
int_codec!(u64);
int_codec!(i8);
int_codec!(i16);
int_codec!(i32);
int_codec!(i64);

macro_rules! vec_codec {
    ($vecname:ident $lenty:ident) => {
        /// Encodable vector container type with a fixed size length tag.
        pub struct $vecname<T> {
            entries: Vec<T>,
        }

        impl<T> $vecname<T> {
            pub fn new(entries: Vec<T>) -> CodecResult<Self> {
                if entries.len() > $lenty::MAX as usize {
                    return Err(CodecError::OversizeContainer);
                }
                Ok(Self { entries })
            }

            pub fn inner(&self) -> &[T] {
                &self.entries
            }

            /// Returns a mut ref to the inner container's slice.
            ///
            /// The number of entries can't be changed as that could permit
            /// making it larger than the permitted size.
            pub fn inner_mut(&mut self) -> &mut [T] {
                &mut self.entries
            }

            pub fn into_inner(self) -> Vec<T> {
                self.entries
            }

            pub fn len(&self) -> usize {
                self.entries.len()
            }

            pub fn is_empty(&self) -> bool {
                self.entries.is_empty()
            }
        }

        impl<T: Codec> Codec for $vecname<T> {
            fn decode(dec: &mut impl Decoder) -> CodecResult<Self> {
                let len = $lenty::decode(dec)?;
                let mut entries = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    let v = T::decode(dec)?;
                    entries.push(v);
                }
                Ok(Self { entries })
            }

            fn encode(&self, enc: &mut impl Encoder) -> CodecResult<()> {
                if self.entries.len() > $lenty::MAX as usize {
                    panic!("dacodec: oversized container");
                }

                let len = self.entries.len() as $lenty;
                len.encode(enc)?;
                for e in &self.entries {
                    e.encode(enc)?;
                }

                Ok(())
            }
        }
    };
}

vec_codec!(SmallVec u8);
vec_codec!(MediumVec u16);
vec_codec!(LargeVec u32);

/// Encodes a codec value into a buffer and returns it.
pub fn encode_to_vec<C: Codec>(v: &C) -> CodecResult<Vec<u8>> {
    let mut buf = Vec::new();
    v.encode(&mut buf)?;
    Ok(buf)
}

/// Consumes a buffer and decodes a codec value from it.
pub fn decode_vec<C: Codec>(buf: Vec<u8>) -> CodecResult<C> {
    let mut cur = BufCursor::new(buf);
    let v = C::decode(&mut cur)?;
    Ok(v)
}
