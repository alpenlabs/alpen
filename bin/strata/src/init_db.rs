//! Database initialization.

use std::{path::Path, sync::Arc};

use strata_db_store_sled::{
    SLED_NAME, SledBackend, SledDbConfig, init_core_dbs, open_sled_database,
};

/// Initialize database backend based on configured features
pub(crate) fn init_database(datadir: &Path, retry_count: u16) -> anyhow::Result<Arc<SledBackend>> {
    let sled_db = open_sled_database(datadir, SLED_NAME)?;
    let retry_delay_ms = 200u64;
    let db_config = SledDbConfig::new_with_constant_backoff(retry_count, retry_delay_ms);
    Ok(init_core_dbs(sled_db.clone(), db_config.clone())?)
}
