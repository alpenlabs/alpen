use std::{path::Path, sync::Arc};

use typed_sled::SledDb;

pub(crate) fn open_sled_database(database_dir: &Path) -> anyhow::Result<Arc<SledDb>> {
    strata_db_store_sled::open_sled_database(database_dir, strata_db_store_sled::SLED_NAME)
}
