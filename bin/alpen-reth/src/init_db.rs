use std::{fs, path::Path, sync::Arc};

use eyre::{eyre, Context, Result};
// Consume the sled dependencies when rocksdb is active to avoid unused crate warnings
#[cfg(feature = "rocksdb")]
use sled as _;
#[cfg(feature = "rocksdb")]
use typed_sled as _;
#[cfg(feature = "rocksdb")]
use {alpen_reth_db::rocksdb::WitnessDB as RocksWitnessDB, rockbound::OptimisticTransactionDB};
#[cfg(all(feature = "sled", not(feature = "rocksdb")))]
use {alpen_reth_db::sled::WitnessDB as SledWitnessDB, typed_sled::SledDb};

// Type aliases for witness database
#[cfg(all(feature = "sled", not(feature = "rocksdb")))]
pub(crate) type WitnessDB = SledWitnessDB;
#[cfg(feature = "rocksdb")]
pub(crate) type WitnessDB = RocksWitnessDB<OptimisticTransactionDB>;

/// Initialize witness database based on configured features
#[cfg(all(feature = "sled", not(feature = "rocksdb")))]
pub(crate) fn init_witness_db(datadir: &Path) -> Result<Arc<WitnessDB>> {
    let database_dir = datadir.join("sled");

    fs::create_dir_all(&database_dir)
        .wrap_err_with(|| format!("creating database directory at {:?}", database_dir))?;

    let sled_db = sled::open(&database_dir).wrap_err("opening sled database")?;

    let typed_sled =
        SledDb::new(sled_db).map_err(|e| eyre!("Failed to create typed sled db: {}", e))?;

    let witness_db = WitnessDB::new(Arc::new(typed_sled))
        .map_err(|e| eyre!("Failed to create witness db: {}", e))?;
    Ok(Arc::new(witness_db))
}

#[cfg(feature = "rocksdb")]
pub(crate) fn init_witness_db(datadir: &Path) -> Result<Arc<WitnessDB>> {
    use strata_db_store_rocksdb::{open_rocksdb_database, ROCKSDB_NAME};

    let database_dir = datadir.join("rocksdb");
    fs::create_dir_all(&database_dir)
        .wrap_err_with(|| format!("creating database directory at {:?}", database_dir))?;

    let rocksdb = open_rocksdb_database(&database_dir, ROCKSDB_NAME)
        .map_err(|e| eyre!("Failed to open rocksdb: {}", e))?;

    let witness_db =
        WitnessDB::new(rocksdb).map_err(|e| eyre!("Failed to create witness db: {}", e))?;
    Ok(Arc::new(witness_db))
}
