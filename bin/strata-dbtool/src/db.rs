use std::{path::Path, sync::Arc};

use strata_cli_common::errors::{DisplayableError, DisplayedError};
#[cfg(feature = "rocksdb")]
use strata_db_store_rocksdb::{
    init_rocksdb_backend, open_rocksdb_database, DbOpsConfig, RocksDbBackend, ROCKSDB_NAME,
};
// Consume the sled dependency when rocksdb is active to avoid unused crate warnings
#[cfg(feature = "rocksdb")]
use strata_db_store_sled as _;
#[cfg(all(feature = "sled", not(feature = "rocksdb")))]
use strata_db_store_sled::{open_sled_database, SledBackend, SledDbConfig, SLED_NAME};

pub(crate) enum DbType {
    #[cfg(all(feature = "sled", not(feature = "rocksdb")))]
    Sled,
    #[cfg(feature = "rocksdb")]
    Rocksdb,
}

impl std::str::FromStr for DbType {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            #[cfg(all(feature = "sled", not(feature = "rocksdb")))]
            "sled" => Ok(DbType::Sled),
            #[cfg(feature = "rocksdb")]
            "rocksdb" | "rocks" => Ok(DbType::Rocksdb),
            other => {
                let available_types = [
                    #[cfg(all(feature = "sled", not(feature = "rocksdb")))]
                    "sled",
                    #[cfg(feature = "rocksdb")]
                    "rocksdb",
                ]
                .join(", ");
                Err(format!("unknown db type '{other}', available types: {available_types}").into())
            }
        }
    }
}

// Type alias for database backend
#[cfg(all(feature = "sled", not(feature = "rocksdb")))]
type DatabaseImpl = SledBackend;
#[cfg(feature = "rocksdb")]
type DatabaseImpl = RocksDbBackend;

/// Returns a boxed trait-object that satisfies all the low-level traits.
pub(crate) fn open_database(
    path: &Path,
    db_type: DbType,
) -> Result<Arc<DatabaseImpl>, DisplayedError> {
    match db_type {
        #[cfg(all(feature = "sled", not(feature = "rocksdb")))]
        DbType::Sled => {
            let sled_db = open_sled_database(path, SLED_NAME)
                .internal_error("Failed to open sled database")?;

            let config = SledDbConfig::new_with_constant_backoff(5, 200);
            let backend = SledBackend::new(sled_db, config)
                .internal_error("Could not open sled backend")
                .map(Arc::new)?;

            Ok(backend)
        }
        #[cfg(feature = "rocksdb")]
        DbType::Rocksdb => {
            let rocksdb = open_rocksdb_database(path, ROCKSDB_NAME)
                .internal_error("Failed to open rocksdb database")?;

            let ops_config = DbOpsConfig::new(5);
            let backend = init_rocksdb_backend(rocksdb, ops_config);

            Ok(backend)
        }
    }
}
