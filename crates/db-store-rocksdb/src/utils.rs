use rockbound::{rocksdb, OptimisticTransactionDB as DB, Schema, SchemaDBOperationsExt};
use strata_db::{errors::DbError, DbResult};

pub fn get_last_idx<T>(db: &DB) -> DbResult<Option<u64>>
where
    T: Schema<Key = u64>,
{
    let mut iterator = db.iter::<T>()?;
    iterator.seek_to_last();
    match iterator.rev().next() {
        Some(res) => {
            let (tip, _) = res?.into_tuple();
            Ok(Some(tip))
        }
        None => Ok(None),
    }
}

pub fn get_first_idx<T>(db: &DB) -> DbResult<Option<u64>>
where
    T: Schema<Key = u64>,
{
    let mut iterator = db.iter::<T>()?;
    match iterator.next() {
        Some(res) => {
            let (tip, _) = res?.into_tuple();
            Ok(Some(tip))
        }
        None => Ok(None),
    }
}

// shim for translating rocksdb error to DbError
pub fn translate_rocksdb_error(err: rocksdb::Error) -> DbError {
    match err.kind() {
        rocksdb::ErrorKind::InvalidArgument => DbError::InvalidArgument,
        rocksdb::ErrorKind::IOError => DbError::IoError,
        rocksdb::ErrorKind::TimedOut => DbError::TimedOut,
        rocksdb::ErrorKind::Aborted => DbError::Aborted,
        rocksdb::ErrorKind::Busy => DbError::Busy,
        _ => DbError::RocksDb(err.to_string()),
    }
}
