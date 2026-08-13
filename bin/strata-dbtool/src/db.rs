use std::{path::Path, sync::Arc};

use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_store_sled::{open_sled_database, SledBackend, SledDbConfig, SLED_NAME};

/// Returns a boxed trait-object that satisfies all the low-level traits.
pub(crate) fn open_database(path: &Path) -> Result<Arc<SledBackend>, DisplayedError> {
    let sled_db =
        open_sled_database(path, SLED_NAME).internal_error("Failed to open sled database")?;

    let config = SledDbConfig::new_with_constant_backoff(5, 200);
    let backend = SledBackend::new(sled_db, config)
        .internal_error("Could not open sled backend")
        .map(Arc::new)?;

    Ok(backend)
}
