use std::{path::Path, sync::Arc};

// Consume unused rocksdb dependencies when both features are enabled (for linting)
#[cfg(all(feature = "sled", feature = "rocksdb"))]
use strata_db_store_rocksdb as _;
#[cfg(all(feature = "rocksdb", not(feature = "sled")))]
use strata_db_store_rocksdb::{
    init_rocksdb_backend, open_rocksdb_database, DbOpsConfig, RocksDbBackend, ROCKSDB_NAME,
};
#[cfg(feature = "sled")]
use strata_db_store_sled::{
    init_core_dbs, open_sled_database, SledBackend, SledDbConfig, SLED_NAME,
};

// Type aliases for database backends
#[cfg(feature = "sled")]
pub(crate) type DatabaseImpl = SledBackend;
#[cfg(all(feature = "rocksdb", not(feature = "sled")))]
pub(crate) type DatabaseImpl = RocksDbBackend;

/// Initialize database backend based on configured features
pub(crate) fn init_database(
    datadir: &Path,
    db_retry_count: u16,
) -> anyhow::Result<Arc<DatabaseImpl>> {
    #[cfg(feature = "sled")]
    {
        let sled_db = open_sled_database(datadir, SLED_NAME)?;
        let retry_delay_ms = 200u64;
        let db_config = SledDbConfig::new_with_constant_backoff(db_retry_count, retry_delay_ms);
        Ok(init_core_dbs(sled_db.clone(), db_config.clone())?)
    }

    #[cfg(all(feature = "rocksdb", not(feature = "sled")))]
    {
        let rocksdb = open_rocksdb_database(datadir, ROCKSDB_NAME)?;
        let ops_config = DbOpsConfig::new(db_retry_count);
        Ok(init_rocksdb_backend(rocksdb, ops_config))
    }
}
