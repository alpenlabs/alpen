use std::path::Path;

pub use sleddb::EeDatabases;

use crate::sleddb;

/// Opens a single sled instance at `<datadir>/sled` and returns all raw
/// database types.
///
/// Callers wrap individual DBs in ops/managers/threadpools as needed. This
/// keeps DB initialization separate from the ops layer.
pub fn init_db_storage(datadir: &Path, db_retry_count: u16) -> eyre::Result<EeDatabases> {
    sleddb::init_database(datadir, db_retry_count)
}
