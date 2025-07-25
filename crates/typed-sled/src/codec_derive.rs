use borsh::{BorshDeserialize, BorshSerialize};

use crate::{CodecError, CodecResult, KeyCodec, Schema, ValueCodec};

// Blanket implementations for borsh derived types. We can later add other implementations with
// feature gates.

/// Marker trait to filter out integers from implementing default Codec
/// NOTE: This should not be derived by integer types as those would be encoded in little endian
/// order by borsh.
/// However, if the default serialization is changed from Borsh to something else which preserves
/// order, then this can be implemented by any trait. Actually this would be redundant.
pub trait DefaultCodecDeriveBorsh {}

// Derive for basic types

// Primitive types (non-integer)
impl DefaultCodecDeriveBorsh for bool {}
impl DefaultCodecDeriveBorsh for String {}
impl DefaultCodecDeriveBorsh for () {}

// Generic standard library containers (only implement if inner types also do)
impl<T: DefaultCodecDeriveBorsh> DefaultCodecDeriveBorsh for Option<T> {}
impl<T: DefaultCodecDeriveBorsh> DefaultCodecDeriveBorsh for Vec<T> {}
impl<T: DefaultCodecDeriveBorsh> DefaultCodecDeriveBorsh for Box<T> {}
impl<T: DefaultCodecDeriveBorsh, E: DefaultCodecDeriveBorsh> DefaultCodecDeriveBorsh
    for Result<T, E>
{
}

// Common tuple types (arity up to 4 for practical coverage)
impl<A: DefaultCodecDeriveBorsh, B: DefaultCodecDeriveBorsh> DefaultCodecDeriveBorsh for (A, B) {}
impl<A: DefaultCodecDeriveBorsh, B: DefaultCodecDeriveBorsh, C: DefaultCodecDeriveBorsh>
    DefaultCodecDeriveBorsh for (A, B, C)
{
}
impl<
    A: DefaultCodecDeriveBorsh,
    B: DefaultCodecDeriveBorsh,
    C: DefaultCodecDeriveBorsh,
    D: DefaultCodecDeriveBorsh,
> DefaultCodecDeriveBorsh for (A, B, C, D)
{
}

// Blanket implementation for KeyCodec
impl<T, S> KeyCodec<S> for T
where
    T: BorshSerialize + BorshDeserialize + DefaultCodecDeriveBorsh,
    S: Schema,
{
    fn encode_key(&self) -> CodecResult<Vec<u8>> {
        borsh::to_vec(self).map_err(CodecError::Deserialization)
    }

    fn decode_key(buf: &[u8]) -> CodecResult<Self> {
        borsh::from_slice(buf).map_err(CodecError::Deserialization)
    }
}

// Blanket implementation for ValueCodec
impl<T, S> ValueCodec<S> for T
where
    T: BorshSerialize + BorshDeserialize + DefaultCodecDeriveBorsh,
    S: Schema,
{
    fn encode_value(&self) -> CodecResult<Vec<u8>> {
        borsh::to_vec(self).map_err(CodecError::Deserialization)
    }

    fn decode_value(buf: &[u8]) -> CodecResult<Self> {
        borsh::from_slice(buf).map_err(CodecError::Deserialization)
    }
}

// Impls for integer types

macro_rules! impl_key_codec_be {
    ($($t:ty),*) => {
        $(
            impl<S: Schema> KeyCodec<S> for $t {
                fn encode_key(&self) -> CodecResult<Vec<u8>> {
                    Ok(self.to_be_bytes().to_vec())
                }

                fn decode_key(buf: &[u8]) -> CodecResult<Self> {
                    const SIZE: usize = std::mem::size_of::<$t>();
                    if buf.len() != SIZE {
                        return Err(CodecError::InvalidLength {
                            expected: SIZE,
                            got: buf.len(),
                        });
                    }

                    let mut bytes = [0u8; SIZE];
                    bytes.copy_from_slice(buf);
                    Ok(<$t>::from_be_bytes(bytes))
                }
            }

            impl<S: Schema> ValueCodec<S> for $t {
                fn encode_value(&self) -> CodecResult<Vec<u8>> {
                    Ok(self.to_be_bytes().to_vec())
                }

                fn decode_value(buf: &[u8]) -> CodecResult<Self> {
                    const SIZE: usize = std::mem::size_of::<$t>();
                    if buf.len() != SIZE {
                        return Err(CodecError::InvalidLength {
                            expected: SIZE,
                            got: buf.len(),
                        });
                    }

                    let mut bytes = [0u8; SIZE];
                    bytes.copy_from_slice(buf);
                    Ok(<$t>::from_be_bytes(bytes))
                }
            }

        )*
    };
}

impl_key_codec_be!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);
