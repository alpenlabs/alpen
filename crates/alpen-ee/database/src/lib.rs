pub mod database;
pub mod error;
mod init;
mod serialization_types;
mod sleddb;
mod storage;

// NOTE: `sled` is gitignored
pub use error::{DbError, DbResult};
pub use init::init_db_storage;
use sleddb as sled;
