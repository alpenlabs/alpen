//! Verifies and imports V0 history for the five explorer RPCs.
mod archive;
mod walk;

use std::path::PathBuf;

use anyhow::{Result, bail, ensure};
use argh::FromArgs;
use strata_identifiers::{Buf32, Epoch, OLBlockId, Slot};
#[cfg(test)]
use {strata_db_types as _, tempfile as _};

/// Verify a disposable, offline copy of an existing v0.3.5 node datadir.
/// Sled requires exclusive, writable access for locking and recovery.
#[derive(FromArgs)]
struct Args {
    /// root of the copied datadir containing sled/strata-client
    #[argh(option)]
    datadir: PathBuf,
    /// expected 32-byte genesis block ID in hex
    #[argh(option, from_str_fn(parse_id))]
    genesis_id: OLBlockId,
    /// expected 32-byte promotion terminal block ID in hex
    #[argh(option, from_str_fn(parse_id))]
    terminal_id: OLBlockId,
    /// slot of the promotion terminal
    #[argh(option)]
    terminal_slot: Slot,
    /// epoch of the promotion terminal
    #[argh(option)]
    terminal_epoch: Epoch,
    /// import into this offline promoted node datadir (requires --source-snapshot)
    #[argh(option)]
    target_datadir: Option<PathBuf>,
    /// source snapshot identifier recorded in the archive manifest
    #[argh(option)]
    source_snapshot: Option<String>,
}

fn parse_id(input: &str) -> Result<OLBlockId, String> {
    input
        .parse::<Buf32>()
        .map(Into::into)
        .map_err(|e| e.to_string())
}

fn main() -> Result<()> {
    let args: Args = argh::from_env();
    let report = match (&args.target_datadir, &args.source_snapshot) {
        (None, None) => walk::walk(&args, |_| Ok(()))?,
        (Some(dir), Some(source)) => {
            ensure!(
                !source.trim().is_empty(),
                "source snapshot must be nonempty"
            );
            archive::import(&args, dir, source)?
        }
        _ => bail!("--target-datadir and --source-snapshot must be supplied together"),
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
