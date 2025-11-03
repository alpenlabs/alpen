mod database;
mod error;
mod init;
mod serialization_types;
mod storage;

#[cfg(feature = "rocksdb")]
mod rocksdb;
#[cfg(feature = "sled")]
mod sled;

pub(crate) use error::DbError;
pub(crate) use init::init_db_storage;

pub(crate) type DbResult<T> = Result<T, DbError>;

// Ensure only one database backend is configured at a time
#[cfg(all(
    feature = "sled",
    feature = "rocksdb",
    not(any(test, debug_assertions))
))]
compile_error!(
    "multiple database backends configured: both 'sled' and 'rocksdb' features are enabled"
);
