use std::{fs, path::Path, sync::Arc};

use eyre::{eyre, Context, Result};
use strata_db_store_rocksdb::{open_rocksdb_database, ROCKSDB_NAME};

use crate::db::rocksdb::db::EeNodeRocksDb;

/// Initialize database based on configured features
pub(crate) fn init_db(datadir: &Path, db_retry_count: u16) -> Result<Arc<EeNodeRocksDb>> {
    let database_dir = datadir.join("sled");

    fs::create_dir_all(&database_dir)
        .wrap_err_with(|| format!("creating database directory at {:?}", database_dir))?;

    let rocksdb = open_rocksdb_database(&database_dir, ROCKSDB_NAME)
        .map_err(|e| eyre!("Failed to open rocksdb: {}", e))?;

    let node_db = EeNodeRocksDb::new(rocksdb, db_retry_count);

    Ok(Arc::new(node_db))
}
