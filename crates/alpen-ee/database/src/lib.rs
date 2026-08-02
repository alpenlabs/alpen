//! Database implementation for Alpen execution environment.

pub mod database;
pub mod error;
mod init;
mod instrumentation;
mod serialization_types;
mod sleddb;
mod storage;

pub use error::{DbError, DbResult};
pub use init::{open_for_node, open_for_offline_tooling, EeDatabases};
pub use sleddb::{BroadcastDbOps, ChunkedEnvelopeOps, EeProverDbSled};
pub use storage::EeNodeStorage;
