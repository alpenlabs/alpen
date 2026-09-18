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
/// Preserves aborted domain errors, contention, I/O categories, and codec failures so callers can
/// distinguish recoverable failures from corruption.
///
/// This is a free function rather than a `From` impl because both [`Error`] and [`DbError`] are
/// foreign to this crate, so the orphan rule forbids the impl here.
pub fn conv_sled_err(err: Error) -> DbError {
    match err.downcast_abort::<DbError>() {
        Ok(db_err) => db_err,
        Err(Error::CodecError(error)) => DbError::CodecError(error.to_string()),
        Err(Error::SledError(error))
        | Err(Error::TransactionError(UnabortableTransactionError::Storage(error))) => {
            conv_sled_storage_err(error)
        }
        Err(Error::TransactionError(UnabortableTransactionError::Conflict)) => DbError::Busy,
        Err(other) => DbError::Other(format!("sled error: {other:?}")),
    }
}

/// Preserves storage failure categories for both transaction and direct tree operations.
pub(crate) fn conv_sled_storage_err(error: SledError) -> DbError {
    match error {
        SledError::Io(error) => DbError::Io {
            kind: error.kind(),
            message: error.to_string(),
        },
        error @ SledError::Corruption { .. } => DbError::Corruption(error.to_string()),
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
