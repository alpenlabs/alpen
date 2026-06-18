//! Thin driver for the one-shot, idempotent Alpen sled schema migration.
//!
//! Upgrades a staging-v2 datadir written by pre-#1849 binaries so that current
//! (post-#1849) binaries can read it. The actual migration logic lives inside
//! the two DB crates (`alpen_ee_database::migrate_ee_db`,
//! `strata_db_store_sled::migrate_ol_db`) so it can reach the `pub(crate)`
//! schemas and record types; this binary only opens the sled instances and
//! invokes them.
//!
//! Both migrations are idempotent (a version-marker key makes a second run a
//! no-op), so it is safe to re-run.

use std::{path::PathBuf, process::ExitCode};

use anyhow::Context;
use argh::FromArgs;
use tracing::{error, info};
use tracing_subscriber::fmt::init as init_tracing;

/// On-disk OL sled sub-path under a node datadir (`<datadir>/sled/<SLED_NAME>`).
/// Matches `strata_db_store_sled::SLED_NAME`.
const OL_SLED_NAME: &str = "strata-client";

/// One-shot, idempotent Alpen sled schema migration.
#[derive(FromArgs, Debug)]
struct Args {
    /// path to the EE sled directory (`<ee-datadir>/sled`). If a node datadir is
    /// passed, append `/sled`.
    #[argh(option)]
    ee_sled: Option<PathBuf>,

    /// path to the OL sled directory (`<ol-datadir>/sled/strata-client`).
    #[argh(option)]
    ol_sled: Option<PathBuf>,

    /// path to an OL *node datadir*; resolves to `<datadir>/sled/strata-client`.
    #[argh(option)]
    ol_datadir: Option<PathBuf>,

    /// path to an EE *node datadir*; resolves to `<datadir>/sled`.
    #[argh(option)]
    ee_datadir: Option<PathBuf>,
}

fn main() -> ExitCode {
    init_tracing();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!(error = %format!("{err:#}"), "migration failed");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    let ee_path = args
        .ee_sled
        .or_else(|| args.ee_datadir.map(|d| d.join("sled")));
    let ol_path = args.ol_sled.or_else(|| {
        args.ol_datadir
            .map(|d| d.join("sled").join(OL_SLED_NAME))
    });

    if ee_path.is_none() && ol_path.is_none() {
        anyhow::bail!(
            "nothing to do: pass at least one of --ee-sled/--ee-datadir or --ol-sled/--ol-datadir"
        );
    }

    if let Some(path) = ee_path {
        info!(path = %path.display(), "migrating EE sled DB");
        let db = sled::open(&path)
            .with_context(|| format!("opening EE sled at {}", path.display()))?;
        let report = alpen_ee_database::migrate_ee_db(&db)
            .map_err(|e| anyhow::anyhow!("EE migration: {e:#}"))?;
        info!(?report, "EE migration done");
    }

    if let Some(path) = ol_path {
        info!(path = %path.display(), "migrating OL sled DB");
        let db = sled::open(&path)
            .with_context(|| format!("opening OL sled at {}", path.display()))?;
        let report = strata_db_store_sled::migrate_ol_db(&db).context("OL migration")?;
        info!(?report, "OL migration done");
    }

    Ok(())
}
