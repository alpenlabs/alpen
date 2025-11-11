use std::{fs, path::Path, sync::Arc};

use alpen_reth_db::sled::WitnessDB as SledWitnessDB;
use eyre::{eyre, Context, Result};
use typed_sled::SledDb;

/// Initialize witness database based on configured features
pub(crate) fn init_witness_db(datadir: &Path) -> Result<Arc<SledWitnessDB>> {
    let database_dir = datadir.join("sled");

    fs::create_dir_all(&database_dir)
        .wrap_err_with(|| format!("creating database directory at {:?}", database_dir))?;

    let sled_db = sled::open(&database_dir).wrap_err("opening sled database")?;

    let typed_sled =
        SledDb::new(sled_db).map_err(|e| eyre!("Failed to create typed sled db: {}", e))?;

    let witness_db = SledWitnessDB::new(Arc::new(typed_sled))
        .map_err(|e| eyre!("Failed to create witness db: {}", e))?;
    Ok(Arc::new(witness_db))
}
