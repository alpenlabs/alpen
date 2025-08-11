use thiserror::Error;

use crate::schema::Schema;

/// Errors that can occur during key/value encoding or decoding.
#[derive(Debug, Error)]
pub enum CodecError {
    /// Key has invalid length for the expected type.
    #[error("invalid key length in '{schema}' (expected {expected} bytes, got {actual})")]
    InvalidKeyLength {
        schema: &'static str,
        expected: usize,
        actual: usize,
    },
    /// Value serialization failed.
    #[error("failed to serialize schema '{schema}' value")]
    SerializationFailed {
        schema: &'static str,
        #[source]
        source: Box<dyn std::error::Error>,
    },
    /// Value deserialization failed.
    #[error("failed to deserialize schema '{schema}' value")]
    DeserializationFailed {
        schema: &'static str,
        #[source]
        source: Box<dyn std::error::Error>,
    },
    /// I/O error during codec operations.
    #[error("io: {0}")]
    IO(#[from] std::io::Error),

    /// Other
    #[error("other: {0}")]
    Other(String),
}

pub type CodecResult<T> = Result<T, CodecError>;

/// Trait for encoding and decoding keys for a specific schema.
pub trait KeyCodec<S: Schema>: Sized {
    /// Encodes the key into bytes.
    fn encode_key(&self) -> CodecResult<Vec<u8>>;
    /// Decodes the key from bytes.
    fn decode_key(buf: &[u8]) -> CodecResult<Self>;
}

/// Trait for encoding and decoding values for a specific schema.
pub trait ValueCodec<S: Schema>: Sized {
    /// Encodes the value into bytes.
    fn encode_value(&self) -> CodecResult<Vec<u8>>;
    /// Decodes the value from bytes.
    fn decode_value(buf: &[u8]) -> CodecResult<Self>;
}

macro_rules! derive_key_codec_for_integers {
    ($($int:ty), *) => {
        $(impl<T: Schema> KeyCodec<T> for $int {
            fn encode_key(&self) -> CodecResult<Vec<u8>> {
                Ok(self.to_be_bytes().into())
            }

            fn decode_key(buf: &[u8]) -> CodecResult<Self> {
                const SIZE: usize = std::mem::size_of::<$int>();
                if buf.len() != SIZE {
                    return Err(CodecError::InvalidKeyLength {
                        schema: T::TREE_NAME.0,
                        expected: SIZE,
                        actual: buf.len(),
                    });
                }
                let mut bytes = [0u8; SIZE];
                bytes.copy_from_slice(buf);
                Ok(<$int>::from_be_bytes(bytes))
            }
        })*
    };
}

derive_key_codec_for_integers!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);
