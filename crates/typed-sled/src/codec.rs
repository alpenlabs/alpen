use crate::schema::Schema;

/// Errors that can occur during key/value encoding or decoding.
#[derive(Debug)]
pub enum CodecError {
    /// Key has invalid length for the expected type.
    InvalidKeyLength {
        schema: &'static str,
        expected: usize,
        actual: usize,
    },
    /// Value serialization failed using borsh.
    SerializationFailed {
        schema: &'static str,
        source: std::io::Error,
    },
    /// Value deserialization failed using borsh.
    DeserializationFailed {
        schema: &'static str,
        source: std::io::Error,
    },
    /// I/O error during codec operations.
    IO(std::io::Error),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::InvalidKeyLength { schema, expected, actual } => {
                write!(f, "Invalid key length for schema '{schema}': expected {expected} bytes, got {actual}")
            }
            CodecError::SerializationFailed { schema, source } => {
                write!(f, "Failed to serialize value for schema '{schema}': {source}")
            }
            CodecError::DeserializationFailed { schema, source } => {
                write!(f, "Failed to deserialize value for schema '{schema}': {source}")
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
            CodecError::SerializationFailed { source, .. } => Some(source),
            CodecError::DeserializationFailed { source, .. } => Some(source),
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
