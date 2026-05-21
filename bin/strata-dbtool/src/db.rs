use std::{path::Path, sync::Arc};

use alpen_ee_database::EeProverDbSled;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_store_sled::{open_sled_database, SledBackend, SledDbConfig, SLED_NAME};
use typed_sled::SledDb;

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

/// Opens the EE prover sled store at `<ee_datadir>/sled`.
///
/// Mirrors the alpen-client's [`alpen_ee_database::init_db_storage`]
/// opener but only constructs the prover-task / chunk-receipt / acct-proof
/// trees — the dbtool has no use for the other EE DBs (witness, broadcast,
/// chunked-envelope, DA context), and skipping them keeps the cold-start
/// surface smaller.
pub(crate) fn open_ee_database(ee_datadir: &Path) -> Result<Arc<EeProverDbSled>, DisplayedError> {
    let database_dir = ee_datadir.join("sled");
    let sled_db = sled::open(&database_dir).map_err(|e| {
        DisplayedError::UserError(
            format!("Failed to open EE sled database at {database_dir:?}"),
            Box::new(e),
        )
    })?;

    let typed_sled =
        Arc::new(SledDb::new(sled_db).internal_error("Could not initialize typed-sled wrapper")?);

    let config = SledDbConfig::new_with_constant_backoff(5, 200);
    let prover_db = EeProverDbSled::new(typed_sled, config)
        .internal_error("Could not open EE prover db")
        .map(Arc::new)?;

    Ok(prover_db)
}
