use std::{fs, path::Path, sync::Arc};

use eyre::{eyre, Context, Result};
use typed_sled::SledDb;

pub(crate) fn open_sled_database(datadir: &Path) -> Result<Arc<SledDb>> {
    let database_dir = datadir.join("sled");

    fs::create_dir_all(&database_dir)
        .wrap_err_with(|| format!("creating database directory at {:?}", database_dir))?;

    let sled_db = sled::open(&database_dir).wrap_err("opening sled database")?;

    let typed_sled =
        SledDb::new(sled_db).map_err(|e| eyre!("Failed to create typed sled db: {}", e))?;

    Ok(Arc::new(typed_sled))
}
