mod database;
mod error;
mod init;
mod serialization_types;
mod sleddb;
mod storage;

// NOTE: `sled` is gitignored
pub(crate) use error::DbError;
pub(crate) use init::init_db_storage;
use sleddb as sled;

pub(crate) type DbResult<T> = Result<T, DbError>;
