use std::{path::Path, sync::Arc};

use threadpool::ThreadPool;

use crate::{sleddb::EeNodeDBSled, storage::EeNodeStorage};

type DatabaseImpl = EeNodeDBSled;

fn init_db(datadir: &Path, db_retry_count: u16) -> eyre::Result<Arc<DatabaseImpl>> {
    super::sleddb::init_db(datadir, db_retry_count)
}

pub fn init_db_storage(datadir: &Path, db_retry_count: u16) -> eyre::Result<EeNodeStorage> {
    let db = init_db(datadir, db_retry_count)?;

    let pool = ThreadPool::new(4);
    Ok(EeNodeStorage::new(pool, db))
}
