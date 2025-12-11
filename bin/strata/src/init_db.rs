//! Database initialization.

use std::sync::Arc;

use anyhow::Result;
use strata_config::ClientConfig;
use strata_db_store_sled::{
    SLED_NAME, SledBackend, SledDbConfig, init_core_dbs, open_sled_database,
};

/// Initialize database backend based on configured features
pub(crate) fn init_database(config: &ClientConfig) -> Result<Arc<SledBackend>> {
    let sled_db = open_sled_database(&config.datadir, SLED_NAME)?;
    let db_config =
        SledDbConfig::new_with_constant_backoff(config.db_retry_count, config.db_retry_delay_ms);
    Ok(init_core_dbs(sled_db, db_config)?)
}
