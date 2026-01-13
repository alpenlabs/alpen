//! Database and storage instrumentation component identifiers.
//!
//! This module defines consistent component naming for tracing instrumentation
//! across the codebase. These constants should be used in `#[instrument]` attributes
//! to ensure consistent filtering and querying.
//!
//! # Component Naming Convention
//!
//! Components use hierarchical naming: `<layer>:<domain>`
//!
//! - **Layer**: The architectural layer (e.g., `storage`, `db`)
//! - **Domain**: The specific area within that layer (e.g., `l1`, `asm`, `chainstate`)
//!
//! # Examples
//!
//! ```rust,ignore
//! use strata_common::instrumentation::components;
//! use tracing::instrument;
//!
//! #[instrument(
//!     skip(self, manifest),
//!     fields(
//!         component = components::STORAGE_L1,
//!         block_id = %manifest.blkid(),
//!         height = manifest.height(),
//!     )
//! )]
//! pub fn put_block_data(&self, manifest: AsmManifest) -> DbResult<()> {
//!     // implementation
//! }
//! ```
//!
//! # Filtering
//!
//! Use `RUST_LOG` to filter by module path:
//!
//! ```bash
//! # Show L1 storage operations at info level
//! RUST_LOG=strata_storage::managers::l1=info
//!
//! # Show DB transactions at debug level
//! RUST_LOG=strata_db_store_sled::config=debug
//!
//! # Show both
//! RUST_LOG=strata_storage::managers::l1=info,strata_db_store_sled::config=debug
//! ```

/// Component identifiers for instrumentation spans.
///
/// Use these constants in `#[instrument]` attributes to ensure consistent
/// component naming across the codebase.
///
/// New constants should be added here as instrumentation is added to other managers.
pub mod components {
    /// L1 block and manifest storage manager operations.
    ///
    /// Tracks L1 block state mutations: put_block_data, extend_canonical_chain,
    /// revert_canonical_chain.
    pub const STORAGE_L1: &str = "storage:l1";

    /// Sled database transaction operations.
    ///
    /// Tracks transaction lifecycle, retries, and conflicts.
    /// Used at `debug` level for detailed troubleshooting.
    pub const DB_SLED_TRANSACTION: &str = "db:sled:transaction";
}
