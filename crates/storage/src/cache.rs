//! Generic cache utility for what we're inserting into the database.

use strata_db::DbError;

/// Wrapper around a LRU cache that handles cache reservations and asynchronously waiting for
/// database operations in the background without keeping a global lock on the cache.
pub(crate) type CacheTable<K, V> = strata_storage_common::cache::CacheTable<K, V, DbError>;
