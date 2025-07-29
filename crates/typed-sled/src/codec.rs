use crate::schema::Schema;

/// Errors that can occur during key/value encoding or decoding.
#[derive(Debug)]
pub enum CodecError {
    /// Unable to deserialize a key because it has a different length than
    /// expected.
    InvalidLength {
        expected: usize,
        got: usize,
    },
    /// Deserialization Error.
    // TODO: make this better
    Deserialization(std::io::Error),

    // TODO: make this better
    Serialization(std::io::Error),
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

            CodecError::Serialization(err) => {
                write!(f, "Serialization error: {err}")
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
