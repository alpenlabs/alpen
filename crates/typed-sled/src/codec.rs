use borsh::{BorshDeserialize, BorshSerialize};

use crate::schema::Schema;

#[derive(Debug)]
pub enum CodecError {
    /// Unable to deserialize a key because it has a different length than
    /// expected.
    InvalidLength { expected: usize, got: usize },
    /// Deserialization Error.
    // TODO: make this better
    Deserialization(std::io::Error),
    /// I/O error.
    IO(std::io::Error),
    // TODO add other
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::InvalidLength { expected, got } => {
                write!(f, "Invalid length: expected {expected}, got {got}")
            }
            CodecError::Deserialization(err) => {
                write!(f, "Deserialization error: {err}")
            }
            CodecError::IO(err) => {
                write!(f, "I/O error: {err}")
            }
        }
    }
}

impl std::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CodecError::Deserialization(err) => Some(err),
            CodecError::IO(err) => Some(err),
            _ => None,
        }
    }
}

pub type CodecResult<T> = Result<T, CodecError>;

pub trait KeyCodec<S: Schema>: Sized {
    fn encode_key(&self) -> CodecResult<Vec<u8>>;
    fn decode_key(buf: &[u8]) -> CodecResult<Self>;
}

pub trait ValueCodec<S: Schema>: Sized {
    fn encode_value(&self) -> CodecResult<Vec<u8>>;
    fn decode_value(buf: &[u8]) -> CodecResult<Self>;
}

// Blanket implementations for borsh derived types. We can later add other implementations with
// feature gates.

// Blanket implementation for KeyCodec
impl<T, S> KeyCodec<S> for T
where
    T: BorshSerialize + BorshDeserialize + Sized,
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
    T: BorshSerialize + BorshDeserialize + Sized,
    S: Schema,
{
    fn encode_value(&self) -> CodecResult<Vec<u8>> {
        borsh::to_vec(self).map_err(CodecError::Deserialization)
    }

    fn decode_value(buf: &[u8]) -> CodecResult<Self> {
        borsh::from_slice(buf).map_err(CodecError::Deserialization)
    }
}
