use std::{path::Path, sync::Arc};

use strata_rocksdb::{init_core_dbs, open_rocksdb_database, CommonDb, DbOpsConfig, ROCKSDB_NAME};

use crate::errors::{DbtoolError, Result};

pub enum DbType {
    Rocksdb,
}

impl std::str::FromStr for DbType {
    type Err = DbtoolError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "rocksdb" => Ok(DbType::Rocksdb),
            other => Err(DbtoolError::Db(format!("unknown db type {other}"))),
        }
    }
}

/// Returns a boxed trait-object that satisfies all the low-level traits.
pub fn open_database(path: &Path, db_type: DbType) -> Result<Arc<CommonDb>> {
    match db_type {
        DbType::Rocksdb => {
            let rbdb = open_rocksdb_database(path, ROCKSDB_NAME)
                .map_err(|e| DbtoolError::Db(e.to_string()))
                .unwrap();
            Ok(init_core_dbs(rbdb, DbOpsConfig::new(3)))
        }
    }
}
