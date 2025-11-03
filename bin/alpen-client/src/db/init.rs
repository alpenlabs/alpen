use std::{path::Path, sync::Arc};

use threadpool::ThreadPool;

use crate::db::{sled::EeNodeDBSled, storage::EeNodeStorage};

#[cfg(feature = "sled")]
type DatabaseImpl = EeNodeDBSled;

fn init_db(datadir: &Path, db_retry_count: u16) -> eyre::Result<Arc<DatabaseImpl>> {
    #[cfg(feature = "sled")]
    {
        super::sled::init_db(datadir, db_retry_count)
    }
    #[cfg(feature = "rocksdb")]
    {
        todo!()
    }
}

pub(crate) fn init_db_storage(datadir: &Path, db_retry_count: u16) -> eyre::Result<EeNodeStorage> {
    let db = init_db(datadir, db_retry_count)?;

    let pool = ThreadPool::new(4);
    Ok(EeNodeStorage::new(pool, db))
}
