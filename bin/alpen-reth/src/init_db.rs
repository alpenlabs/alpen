use std::{fs, path::Path, sync::Arc};

use eyre::{eyre, Context, Result};
#[cfg(all(feature = "rocksdb", not(feature = "sled")))]
use {alpen_reth_db::rocksdb::WitnessDB as RocksWitnessDB, rockbound::OptimisticTransactionDB};
#[cfg(feature = "sled")]
use {alpen_reth_db::sled::WitnessDB as SledWitnessDB, typed_sled::SledDb};
// Consume unused rocksdb dependencies when both features are enabled (for linting)
#[cfg(all(feature = "sled", feature = "rocksdb"))]
use {rockbound as _, strata_db_store_rocksdb as _};

// Type aliases for witness database
#[cfg(feature = "sled")]
pub(crate) type WitnessDB = SledWitnessDB;
#[cfg(all(feature = "rocksdb", not(feature = "sled")))]
pub(crate) type WitnessDB = RocksWitnessDB<OptimisticTransactionDB>;

/// Initialize witness database based on configured features
#[cfg(feature = "sled")]
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

#[cfg(all(feature = "rocksdb", not(feature = "sled")))]
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
