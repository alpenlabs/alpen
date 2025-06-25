use std::{path::Path, sync::Arc};

use strata_rocksdb::{init_core_dbs, open_rocksdb_database, CommonDb, DbOpsConfig, ROCKSDB_NAME};

use crate::errors::{DisplayableError, DisplayedError};

pub(crate) enum DbType {
    Rocksdb,
}

impl std::str::FromStr for DbType {
    type Err = DisplayedError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "rocksdb" | "rocks" => Ok(DbType::Rocksdb),
            other => Err(DisplayedError::UserError(
                format!("unknown db type {other}"),
                Box::new(()),
            )),
        }
    }
}

/// Returns a boxed trait-object that satisfies all the low-level traits.
pub(crate) fn open_database(path: &Path, db_type: DbType) -> Result<Arc<CommonDb>, DisplayedError> {
    match db_type {
        DbType::Rocksdb => {
            let rbdb = open_rocksdb_database(path, ROCKSDB_NAME)
                .internal_error("Failed to open rocksdb database")?;
            Ok(init_core_dbs(rbdb, DbOpsConfig::new(3)))
        }
    }
}
