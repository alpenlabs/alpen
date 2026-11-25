//! Offline management of the V0 explorer history trees.
mod import;
mod walk;

use std::path::PathBuf;

use anyhow::{Result, ensure};
use clap::{Args, Parser, Subcommand};
use strata_identifiers::{Buf32, Epoch, EpochCommitment, OLBlockId, Slot};
#[cfg(test)]
use {strata_db_types as _, strata_predicate as _, tempfile as _};

/// Verify or manage V0 explorer history on an offline node.
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Walk and verify the V0 source.
    Verify(Source),
    /// Write verified V0 history into a stopped promoted node.
    Import {
        #[command(flatten)]
        source: Source,
        #[arg(long)]
        target_datadir: PathBuf,
        /// Identifier of the V0 backup.
        #[arg(long)]
        source_snapshot: String,
        /// Full Git commit of the tagged importer build.
        #[arg(long, value_parser = parse_revision)]
        importer_revision: String,
    },
    /// Unpublish and remove only the six archive trees on a stopped promoted node.
    Remove {
        #[arg(long)]
        target_datadir: PathBuf,
        #[command(flatten)]
        terminal: Terminal,
    },
}

#[derive(Args)]
struct Source {
    /// root of the copied datadir containing sled/strata-client
    #[arg(long)]
    datadir: PathBuf,
    /// expected 32-byte genesis block ID in hex
    #[arg(long, value_parser = parse_id)]
    genesis_id: OLBlockId,
    #[command(flatten)]
    terminal: Terminal,
}

#[derive(Args)]
struct Terminal {
    /// expected 32-byte promotion terminal block ID in hex
    #[arg(long, value_parser = parse_id)]
    terminal_id: OLBlockId,
    /// slot of the promotion terminal
    #[arg(long)]
    terminal_slot: Slot,
    /// epoch of the promotion terminal
    #[arg(long)]
    terminal_epoch: Epoch,
}

impl Terminal {
    fn commitment(&self) -> EpochCommitment {
        EpochCommitment::new(self.terminal_epoch, self.terminal_slot, self.terminal_id)
    }
}

fn parse_revision(input: &str) -> Result<String> {
    ensure!(
        input.len() == 40 && input.bytes().all(|b| b.is_ascii_hexdigit()),
        "expected a full Git commit"
    );
    Ok(input.to_ascii_lowercase())
}
fn parse_id(input: &str) -> Result<OLBlockId, String> {
    input
        .parse::<Buf32>()
        .map(Into::into)
        .map_err(|e| e.to_string())
}

fn main() -> Result<()> {
    let report = match Cli::parse().command {
        Command::Verify(source) => walk::walk(&source, |_| Ok(()))?,
        Command::Import {
            source,
            target_datadir,
            source_snapshot,
            importer_revision,
        } => {
            ensure!(
                !source_snapshot.trim().is_empty(),
                "source snapshot must be nonempty"
            );
            import::import(
                &source,
                &target_datadir,
                &source_snapshot,
                &importer_revision,
            )?
        }
        Command::Remove {
            target_datadir,
            terminal,
        } => {
            import::remove(&target_datadir, terminal.commitment())?;
            println!("V0 archive removed");
            return Ok(());
        }
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(test)]
#[path = "../tests/support/fixture.rs"]
mod crash_fixture;
