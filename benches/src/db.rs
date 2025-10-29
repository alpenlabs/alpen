//! Database utilities for benchmarking.
//!
//! This module provides common utilities and data generators for benchmarking
//! database operations with feature-gated support for different backends.

use std::sync::Arc;

use tempfile::TempDir;

// Compile-time check to ensure at least one database backend is enabled
#[cfg(all(feature = "db", not(feature = "sled")))]
compile_error!("Database benchmarks require at least one backend feature: 'sled'");

/// `Sled` backend support.
#[cfg(feature = "sled")]
pub mod sled {
    use strata_db_store_sled::{open_sled_database, SledDbConfig};
    use typed_sled::SledDb;

    use super::*;

    /// Creates a temporary `Sled` database instance for benchmarking.
    pub fn create_temp_sled() -> (Arc<SledDb>, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let db = open_sled_database(temp_dir.path(), "benchmark_db")
            .expect("Failed to open Sled database");
        (db, temp_dir)
    }

    /// Default database operations configuration for `Sled` benchmarks.
    pub fn default_sled_ops_config() -> SledDbConfig {
        SledDbConfig::test()
    }
}

#[cfg(feature = "sled")]
pub use sled::*;

/// Different database backends for benchmarking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseBackend {
    /// `Sled` database backend.
    #[cfg(feature = "sled")]
    Sled,
}

impl DatabaseBackend {
    /// Get all available backends based on enabled features.
    #[expect(
        clippy::vec_init_then_push,
        reason = "highly complicated feature-gating"
    )]
    pub fn available_backends() -> Vec<DatabaseBackend> {
        let mut backends = Vec::new();
        #[cfg(feature = "sled")]
        backends.push(DatabaseBackend::Sled);
        backends
    }

    /// Get the name of the backend as a string.
    pub fn name(&self) -> &'static str {
        match self {
            #[cfg(feature = "sled")]
            DatabaseBackend::Sled => "sled",
        }
    }
}

/// Macro to generate benchmarks for all available database backends.
#[macro_export]
macro_rules! bench_all_backends {
    ($benchmark_name:ident, $bench_impl_fn:ident) => {
        pub fn $benchmark_name(c: &mut criterion::Criterion) {
            for backend in $crate::db::DatabaseBackend::available_backends() {
                $bench_impl_fn(backend, c);
            }
        }
    };
}
