use std::{path::Path, sync::Arc};

use alpen_ee_database::{init_db_storage, EeDatabases, EeProverDbSled};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_store_sled::{
    chunked_envelope::L1ChunkedEnvelopeDBSled, open_sled_database, SledBackend, SledDbConfig,
    SLED_NAME,
};
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

/// Opens the EE prover sled store at `<datadir>/sled`.
///
/// `datadir` is expected to be the alpen-client's `--datadir`. This opens
/// the same sled directory as [`alpen_ee_database::init_db_storage`], but
/// constructs only the prover-task / chunk-receipt / acct-proof trees. The
/// dbtool's prover commands read nothing else, so opening the other EE DBs
/// (node, witness, broadcast, chunked-envelope, DA context) would be wasted
/// work.
pub(crate) fn open_ee_prover_database(
    datadir: &Path,
) -> Result<Arc<EeProverDbSled>, DisplayedError> {
    let database_dir = datadir.join("sled");
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

/// Opens the full EE sled store at `<datadir>/sled`.
///
/// Use this for commands that need node-chain state in addition to the prover
/// trees. The narrower [`open_ee_prover_database`] helper stays in place for
/// receipt/task-only commands so those commands do not construct unrelated DB
/// wrappers.
pub(crate) fn open_full_ee_database(datadir: &Path) -> Result<EeDatabases, DisplayedError> {
    init_db_storage(datadir, 5).internal_error("Could not open full EE database")
}

/// Opens the EE chunked-envelope sled store at `<datadir>/sled`.
///
/// `datadir` is expected to be the alpen-client's `--datadir`. Like
/// [`open_ee_prover_database`], this opens the same sled directory as
/// [`alpen_ee_database::init_db_storage`], but constructs only the
/// chunked-envelope tree. The `ee-da-inspect` command reads nothing else, so
/// opening the other EE DBs (node, witness, broadcast, prover, DA context)
/// would be wasted work.
pub(crate) fn open_ee_chunked_envelope_database(
    datadir: &Path,
) -> Result<Arc<L1ChunkedEnvelopeDBSled>, DisplayedError> {
    let database_dir = datadir.join("sled");
    let sled_db = sled::open(&database_dir).map_err(|e| {
        DisplayedError::UserError(
            format!("Failed to open EE sled database at {database_dir:?}"),
            Box::new(e),
        )
    })?;

    let typed_sled =
        Arc::new(SledDb::new(sled_db).internal_error("Could not initialize typed-sled wrapper")?);

    let config = SledDbConfig::new_with_constant_backoff(5, 200);
    let chunked_envelope_db = L1ChunkedEnvelopeDBSled::new(typed_sled, config)
        .internal_error("Could not open EE chunked-envelope db")
        .map(Arc::new)?;

    Ok(chunked_envelope_db)
}
