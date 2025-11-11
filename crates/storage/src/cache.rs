//! Cache utility specialized for strata-storage with `DbError`.
//!
//! This module provides a type alias for `CacheTable` that uses `DbError` from `strata-db`.
//! It is a specialization of the generic `CacheTable` from `strata-storage-common`.
//!
//! For caches with custom error types, use `strata_storage_common::cache::CacheTable` directly.

use strata_db_types::DbError;

/// Type alias for `CacheTable` using `DbError` as the error type.
///
/// This is a specialization of the generic `CacheTable<K, V, E>` from `strata-storage-common`
/// with `E = DbError`. It provides a convenient cache implementation for database operations
/// that use the standard `DbError` type.
///
/// For custom error types, use `strata_storage_common::cache::CacheTable` directly.
pub(crate) type CacheTable<K, V> = strata_storage_common::cache::CacheTable<K, V, DbError>;
