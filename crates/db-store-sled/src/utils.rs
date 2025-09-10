use strata_db::errors::DbError;
use typed_sled::tree::SledTransactionalTree;

pub fn second<A, B>((_, b): (A, B)) -> B {
    b
}

pub fn first<A, B>((a, _): (A, B)) -> A {
    a
}

/// Converts any error that implements Display and Debug into a DbError::Other
pub fn to_db_error<E: std::fmt::Display + std::fmt::Debug>(e: E) -> DbError {
    DbError::Other(e.to_string())
}

/// Find next available ID starting from the given ID, checking for conflicts within a transaction
pub fn find_next_available_id<K, V, S>(
    tree: &SledTransactionalTree<S>,
    start_id: K,
) -> Result<K, typed_sled::error::Error>
where
    K: Clone + std::ops::Add<u64, Output = K>,
    S: typed_sled::Schema<Key = K, Value = V>,
{
    let mut next_id = start_id;
    while tree.get(&next_id)?.is_some() {
        next_id = next_id + 1;
    }
    Ok(next_id)
}
