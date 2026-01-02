use std::{fmt, ops};

use strata_db_types::errors::DbError;
use typed_sled::{Schema, error::Error, tree::SledTransactionalTree};

pub fn second<A, B>((_, b): (A, B)) -> B {
    b
}

pub fn first<A, B>((a, _): (A, B)) -> A {
    a
}

/// Converts any error that implements Display and Debug into a DbError::Other
pub fn to_db_error<E: fmt::Display + fmt::Debug>(e: E) -> DbError {
    DbError::Other(e.to_string())
}

/// Find next available ID starting from the given ID, checking for conflicts within a transaction
pub fn find_next_available_id<K, V, S>(
    tree: &SledTransactionalTree<S>,
    start_id: K,
) -> Result<K, Error>
where
    K: Clone + ops::Add<u64, Output = K>,
    S: Schema<Key = K, Value = V>,
{
    let mut next_id = start_id;
    while tree.get(&next_id)?.is_some() {
        next_id = next_id + 1;
    }
    Ok(next_id)
}
