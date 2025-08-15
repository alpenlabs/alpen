use std::{path::Path, sync::Arc};

use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::DatabaseBackend;
use strata_db_store_sled::{open_sled_database, SledBackend, SledDbConfig, SLED_NAME};

pub(crate) enum DbType {
    Sled,
}

impl std::str::FromStr for DbType {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "sled" => Ok(DbType::Sled),
            other => Err(format!("unknown db type {other}").into()),
        }
    }
}

/// Returns a boxed trait-object that satisfies all the low-level traits.
pub(crate) fn open_database(
    path: &Path,
    db_type: DbType,
) -> Result<Arc<impl DatabaseBackend>, DisplayedError> {
    match db_type {
        DbType::Sled => {
            let sled_db = open_sled_database(path, SLED_NAME)
                .internal_error("Failed to open sled database")?;

            let config = SledDbConfig::new_with_constant_backoff(5, 200);
            let backend = SledBackend::new(sled_db, config)
                .internal_error("Could not open sled backend")
                .map(Arc::new)?;

            Ok(backend)
        }
    }
}
