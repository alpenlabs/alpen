use std::{fs, path::Path, sync::Arc};

use anyhow::Context;
use strata_db_types::DbResult;
use typed_sled::SledDb;

use crate::{SledBackend, SledDbConfig};

// Opens sled database instance from datadir
pub fn open_sled_database(datadir: &Path, dbname: &'static str) -> anyhow::Result<Arc<SledDb>> {
    let mut database_dir = datadir.to_path_buf();
    database_dir.push("sled");
    database_dir.push(dbname);

    if !database_dir.exists() {
        fs::create_dir_all(&database_dir)?;
    }

    let sled_db = sled::open(&database_dir).context("opening sled database")?;

    let db =
        SledDb::new(sled_db).map_err(|e| anyhow::anyhow!("Failed to create sled db: {}", e))?;
    Ok(Arc::new(db))
}

pub fn init_core_dbs(sled_db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Arc<SledBackend>> {
    SledBackend::new(sled_db, config).map(Arc::new)
}
