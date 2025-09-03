//! Database utilities for benchmarking.
//!
//! This module provides common utilities and data generators for benchmarking
//! database operations, including setup helpers and mock data generation.

use std::sync::Arc;

use strata_db_store_rocksdb::{open_rocksdb_database, DbOpsConfig};
use tempfile::TempDir;

/// Creates a temporary `RocksDB` instance for benchmarking.
pub fn create_temp_rocksdb() -> (Arc<rockbound::OptimisticTransactionDB>, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    let db =
        open_rocksdb_database(temp_dir.path(), "benchmark_db").expect("Failed to open RocksDB");

    (db, temp_dir)
}

/// Default database operations configuration for benchmarks.
pub fn default_db_ops_config() -> DbOpsConfig {
    DbOpsConfig::new(10) // 10 retries for benchmarks
}
