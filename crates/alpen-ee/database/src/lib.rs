//! Database implementation for Alpen execution environment.

pub mod database;
pub mod error;
mod init;
mod instrumentation;
pub mod migration;
#[cfg(test)]
mod migration_boot_harness;
#[cfg(test)]
mod migration_probe;
mod serialization_types;
mod sleddb;
mod storage;

pub use error::{DbError, DbResult};
pub use init::{init_db_storage, EeDatabases};
pub use migration::{migrate_ee_db, EeMigrationReport, EE_SCHEMA_VERSION};
pub use sleddb::{BroadcastDbOps, ChunkedEnvelopeOps, EeProverDbSled};
pub use storage::EeNodeStorage;
