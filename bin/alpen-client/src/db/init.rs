use std::{path::Path, sync::Arc};

use threadpool::ThreadPool;

#[cfg(feature = "rocksdb")]
use crate::db::rocksdb::EeNodeRocksDb;
#[cfg(feature = "sled")]
use crate::db::sled::EeNodeDBSled;
use crate::db::storage::EeNodeStorage;

#[cfg(feature = "sled")]
type DatabaseImpl = EeNodeDBSled;
#[cfg(all(feature = "rocksdb", not(feature = "sled")))]
type DatabaseImpl = EeNodeRocksDb;

fn init_db(datadir: &Path, db_retry_count: u16) -> eyre::Result<Arc<DatabaseImpl>> {
    #[cfg(feature = "sled")]
    {
        super::sled::init_db(datadir, db_retry_count)
    }
    #[cfg(all(feature = "rocksdb", not(feature = "sled")))]
    {
        super::rocksdb::init_db(datadir, db_retry_count)
    }
}

pub(crate) fn init_db_storage(datadir: &Path, db_retry_count: u16) -> eyre::Result<EeNodeStorage> {
    let db = init_db(datadir, db_retry_count)?;

    let pool = ThreadPool::new(4);
    Ok(EeNodeStorage::new(pool, db))
}
