use std::{path::Path, sync::Arc};

#[cfg(feature = "rocksdb")]
use strata_db_store_rocksdb::{
    init_rocksdb_backend, open_rocksdb_database, DbOpsConfig, RocksDbBackend, ROCKSDB_NAME,
};
// Consume the sled dependency when rocksdb is active to avoid unused crate warnings
#[cfg(feature = "rocksdb")]
use strata_db_store_sled as _;
#[cfg(all(feature = "sled", not(feature = "rocksdb")))]
use strata_db_store_sled::{
    init_core_dbs, open_sled_database, SledBackend, SledDbConfig, SLED_NAME,
};

// Type aliases for database backends
#[cfg(all(feature = "sled", not(feature = "rocksdb")))]
pub(crate) type DatabaseImpl = SledBackend;
#[cfg(feature = "rocksdb")]
pub(crate) type DatabaseImpl = RocksDbBackend;

/// Initialize database backend based on configured features
pub(crate) fn init_database(
    datadir: &Path,
    db_retry_count: u16,
) -> anyhow::Result<Arc<DatabaseImpl>> {
    #[cfg(all(feature = "sled", not(feature = "rocksdb")))]
    {
        let sled_db = open_sled_database(datadir, SLED_NAME)?;
        let retry_delay_ms = 200u64;
        let db_config = SledDbConfig::new_with_constant_backoff(db_retry_count, retry_delay_ms);
        Ok(init_core_dbs(sled_db.clone(), db_config.clone())?)
    }

    #[cfg(feature = "rocksdb")]
    {
        let rocksdb = open_rocksdb_database(datadir, ROCKSDB_NAME)?;
        let ops_config = DbOpsConfig::new(db_retry_count);
        Ok(init_rocksdb_backend(rocksdb, ops_config))
    }
}
