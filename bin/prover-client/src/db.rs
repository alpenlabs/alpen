use std::{fs, path::Path, sync::Arc};

use rockbound::rocksdb;

pub(crate) fn open_rocksdb_database(
    database_dir: &Path,
) -> anyhow::Result<Arc<rockbound::OptimisticTransactionDB>> {
    let mut database_dir = database_dir.to_path_buf();
    database_dir.push("rocksdb");

    if !database_dir.exists() {
        fs::create_dir_all(&database_dir)?;
    }

    let dbname = strata_rocksdb::ROCKSDB_NAME;
    let cfs = strata_rocksdb::PROVER_COLUMN_FAMILIES;
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let rbdb = rockbound::OptimisticTransactionDB::open(
        &database_dir,
        dbname,
        cfs.iter().map(|s| s.to_string()),
        &opts,
    )?;

    Ok(Arc::new(rbdb))
}
