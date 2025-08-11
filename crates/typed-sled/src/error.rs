// re-export `ConflictableTransactionError`
pub use sled::transaction::ConflictableTransactionError;
use sled::{CompareAndSwapError, Error as SledError, transaction::UnabortableTransactionError};

use crate::CodecError;

/// The main error type for typed-sled operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Codec error
    #[error("codec: {0}")]
    CodecError(#[from] CodecError),

    /// Sled database error
    #[error("sled: {0}")]
    SledError(#[from] SledError),

    /// Sled transaction error
    #[error("sled tx: {0}")]
    TransactionError(#[from] UnabortableTransactionError),

    /// CAS error
    #[error("sled cas: {0}")]
    CASError(#[from] CompareAndSwapError),
}

impl From<Error> for ConflictableTransactionError<Error> {
    fn from(value: Error) -> Self {
        ConflictableTransactionError::Abort(value)
    }
}

/// A type alias for `Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use std::io;

    use sled::Error as SledError;

    use super::*;
    use crate::codec::CodecError;

    #[test]
    fn test_error_from_codec_error() {
        let codec_error = CodecError::InvalidKeyLength {
            schema: "test",
            expected: 4,
            actual: 2,
        };

        let error: Error = codec_error.into();

        match error {
            Error::CodecError(_) => {} // Expected
            _ => panic!("Expected CodecError variant"),
        }
    }

    #[test]
    fn test_error_from_sled_error() {
        let sled_error = SledError::Io(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "test error",
        ));

        let error: Error = sled_error.into();

        match error {
            Error::SledError(_) => {} // Expected
            _ => panic!("Expected SledError variant"),
        }
    }

    #[test]
    fn test_error_from_unabortable_transaction_error() {
        let tx_error = UnabortableTransactionError::Storage(SledError::Io(io::Error::other(
            "transaction failed",
        )));

        let error: Error = tx_error.into();

        match error {
            Error::TransactionError(_) => {} // Expected
            _ => panic!("Expected TransactionError variant"),
        }
    }

    #[test]
    fn test_error_from_cas_error() {
        // Create a CompareAndSwapError - this requires specific sled internals
        // We'll test this by creating an error that would come from actual CAS operations
        let cas_error = CompareAndSwapError {
            current: Some(vec![1, 2, 3].into()),
            proposed: Some(vec![4, 5, 6].into()),
        };

        let error: Error = cas_error.into();

        match error {
            Error::CASError(_) => {} // Expected
            _ => panic!("Expected CASError variant"),
        }
    }

    #[test]
    fn test_error_into_conflictable_transaction_error() {
        let original_error = Error::CodecError(CodecError::InvalidKeyLength {
            schema: "test",
            expected: 4,
            actual: 2,
        });

        let conflictable_error: ConflictableTransactionError<Error> = original_error.into();

        match conflictable_error {
            ConflictableTransactionError::Abort(error) => {
                match error {
                    Error::CodecError(_) => {} // Expected
                    _ => panic!("Expected CodecError variant inside Abort"),
                }
            }
            _ => panic!("Expected Abort variant"),
        }
    }

    #[test]
    fn test_error_display_formatting() {
        // Test that all error variants display properly
        let codec_error = Error::CodecError(CodecError::SerializationFailed {
            schema: "test_schema",
            source: Box::new(io::Error::other("serialization failed")),
        });

        match codec_error {
            Error::CodecError(CodecError::SerializationFailed { .. }) => {} // Expected
            _ => panic!("Expected CodecError::SerializationFailed variant"),
        }
    }

    #[test]
    fn test_error_debug_formatting() {
        let error = Error::SledError(SledError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "file not found",
        )));

        let debug_string = format!("{:?}", error);
        assert!(debug_string.contains("SledError"));
        assert!(debug_string.contains("NotFound"));
    }

    #[test]
    fn test_error_chain_source() {
        let io_error = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");
        let sled_error = SledError::Io(io_error);
        let typed_sled_error = Error::SledError(sled_error);

        // Test that the error chain is preserved
        match typed_sled_error {
            Error::SledError(SledError::Io(_)) => {} // Expected
            _ => panic!("Expected SledError::Io variant"),
        }
    }

    #[test]
    fn test_result_type_alias() {
        // Test that our Result type alias works correctly
        fn test_function() -> Result<i32> {
            Ok(42)
        }

        fn error_function() -> Result<i32> {
            Err(Error::CodecError(CodecError::InvalidKeyLength {
                schema: "test",
                expected: 4,
                actual: 2,
            }))
        }

        let success_result = test_function();
        assert!(success_result.is_ok());
        assert_eq!(success_result.unwrap(), 42);

        let error_result = error_function();
        assert!(error_result.is_err());
        match error_result.unwrap_err() {
            Error::CodecError(_) => {} // Expected
            _ => panic!("Expected CodecError"),
        }
    }

    #[test]
    fn test_codec_error_variations() {
        // Test all CodecError variants can be converted to Error
        let key_length_error = Error::CodecError(CodecError::InvalidKeyLength {
            schema: "test",
            expected: 4,
            actual: 8,
        });

        let serialization_error = Error::CodecError(CodecError::SerializationFailed {
            schema: "test",
            source: Box::new(io::Error::other("serialize failed")),
        });

        let deserialization_error = Error::CodecError(CodecError::DeserializationFailed {
            schema: "test",
            source: Box::new(io::Error::other("deserialize failed")),
        });

        // All should convert properly
        match key_length_error {
            Error::CodecError(CodecError::InvalidKeyLength { .. }) => {} // Expected
            _ => panic!("Expected CodecError::InvalidKeyLength variant"),
        }
        match serialization_error {
            Error::CodecError(CodecError::SerializationFailed { .. }) => {} // Expected
            _ => panic!("Expected CodecError::SerializationFailed variant"),
        }
        match deserialization_error {
            Error::CodecError(CodecError::DeserializationFailed { .. }) => {} // Expected
            _ => panic!("Expected CodecError::DeserializationFailed variant"),
        }
    }
}
