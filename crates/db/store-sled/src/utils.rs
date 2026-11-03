use std::ops;

use sled::Error as SledError;
use sled::transaction::UnabortableTransactionError;
use strata_db_types::errors::DbError;
use typed_sled::error::Error;
use typed_sled::tree::SledTransactionalTree;
use typed_sled::{Schema, ValueCodec};

pub fn second<A, B>((_, b): (A, B)) -> B {
    b
}

pub fn first<A, B>((a, _): (A, B)) -> A {
    a
}

/// Converts a typed-sled [`Error`] into a [`DbError`].
///
/// If the error wraps an aborted `DbError`, the original variant is recovered so callers can match
/// on it rather than on a stringified payload. Storage I/O errors retain their error kind.
///
/// This is a free function rather than a `From` impl because both [`Error`] and [`DbError`] are
/// foreign to this crate, so the orphan rule forbids the impl here.
pub fn conv_sled_err(err: Error) -> DbError {
    match err.downcast_abort::<DbError>() {
        Ok(db_err) => db_err,
        Err(Error::SledError(error))
        | Err(Error::TransactionError(UnabortableTransactionError::Storage(error))) => {
            conv_sled_storage_err(error)
        }
        Err(other) => DbError::Other(format!("sled error: {other:?}")),
    }
}

/// Preserves storage I/O kinds without making other sled failures retryable.
pub(crate) fn conv_sled_storage_err(err: SledError) -> DbError {
    match err {
        SledError::Io(error) => DbError::Io {
            kind: error.kind(),
            message: error.to_string(),
        },
        other => DbError::Other(format!("sled storage: {other:?}")),
    }
}

/// Find next available ID starting from the given ID, checking for conflicts within a transaction
pub fn find_next_available_id<K, V, S>(
    tree: &SledTransactionalTree<S>,
    start_id: K,
) -> Result<K, Error>
where
    K: Clone + ops::Add<u64, Output = K>,
    S: Schema<Key = K, Value = V>,
    V: ValueCodec<S>,
{
    let mut next_id = start_id;
    while tree.get(&next_id)?.is_some() {
        next_id = next_id + 1;
    }
    Ok(next_id)
}

#[cfg(test)]
mod tests {
    use std::io::{Error as IoError, ErrorKind};

    use super::*;

    #[test]
    fn io_kind_survives_typed_sled_wrappers() {
        let io_error = || SledError::Io(IoError::new(ErrorKind::TimedOut, "read timed out"));

        for error in [
            Error::SledError(io_error()),
            Error::TransactionError(UnabortableTransactionError::Storage(io_error())),
        ] {
            let converted = conv_sled_err(error);
            assert!(converted.is_retryable());
            assert!(matches!(
                converted,
                DbError::Io { kind: ErrorKind::TimedOut, message } if message == "read timed out"
            ));
        }
    }

    #[test]
    fn aborted_domain_errors_keep_their_classification() {
        for error in [DbError::Busy, DbError::Other("missing predecessor".into())] {
            let retryable = error.is_retryable();
            let message = error.to_string();
            let converted = conv_sled_err(Error::abort(error));

            assert_eq!(converted.is_retryable(), retryable);
            assert_eq!(converted.to_string(), message);
        }
    }
}
