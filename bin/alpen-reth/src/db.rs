use std::{fs, path::PathBuf, sync::Arc};

use eyre::{eyre, Context};
use rockbound::{rocksdb, DB};

pub(crate) fn open_rocksdb_database(datadir: PathBuf) -> eyre::Result<Arc<DB>> {
    let mut database_dir = datadir;
    database_dir.push("rocksdb");

    if !database_dir.exists() {
        fs::create_dir_all(&database_dir)?;
    }

    let dbname = alpen_reth_db::rocksdb::ROCKSDB_NAME;
    let cfs = alpen_reth_db::rocksdb::STORE_COLUMN_FAMILIES;
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let rbdb = DB::open(
        &database_dir,
        dbname,
        cfs.iter().map(|s| s.to_string()),
        &opts,
    )
    // convert from anyhow -> eyre
    .map_err(|err| eyre!(Box::new(err)))
    .context("opening database")?;

    Ok(Arc::new(rbdb))
}
