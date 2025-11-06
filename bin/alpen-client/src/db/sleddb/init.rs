use std::{fs, path::Path, sync::Arc};

use eyre::{eyre, Context, Result};
use strata_db_store_sled::SledDbConfig;
use typed_sled::SledDb;

use crate::db::sled::EeNodeDBSled;

/// Initialize database based on configured features
pub(crate) fn init_db(datadir: &Path, db_retry_count: u16) -> Result<Arc<EeNodeDBSled>> {
    let database_dir = datadir.join("sled");

    fs::create_dir_all(&database_dir)
        .wrap_err_with(|| format!("creating database directory at {:?}", database_dir))?;

    let sled_db = sled::open(&database_dir).wrap_err("opening sled database")?;

    let typed_sled =
        SledDb::new(sled_db).map_err(|e| eyre!("Failed to create typed sled db: {}", e))?;

    let retry_delay_ms = 200u64;
    let db_config = SledDbConfig::new_with_constant_backoff(db_retry_count, retry_delay_ms);
    let node_db = EeNodeDBSled::new(Arc::new(typed_sled), db_config)
        .map_err(|e| eyre!("Failed to create node db: {}", e))?;
    Ok(Arc::new(node_db))
}
