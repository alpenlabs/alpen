//! Binary entry‑point for the offline Alpen database tool.
//! Parses CLI arguments with **Clap** and delegates to the `alpen_dbtool` lib.
mod cmd;
mod db;
mod errors;

use std::str::FromStr;

use clap::Parser;

use crate::{
    cmd::{
        alpen::get_alpen_block, chainstate::reset_chainstate, checkpoint::get_checkpoint_data,
        epoch::get_epoch_summary, syncinfo::get_syncinfo, Cli, Command,
    },
    db::{open_database, DbType},
};

fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // // Safety: ensure node isn’t running by locking datadir.
    // let _guard = acquire_lock(&cli.datadir)?;

    let db_type = DbType::from_str(&cli.db_type).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    let db = open_database(&cli.datadir, db_type).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    let result = match cli.cmd {
        Command::GetAlpenBlock(args) => get_alpen_block(db, args),
        Command::GetCheckpointData(args) => get_checkpoint_data(db, args),
        Command::GetEpochSummary(args) => get_epoch_summary(db, args),
        Command::GetSyncinfo(args) => get_syncinfo(db, args),
        Command::ResetChainstate(args) => reset_chainstate(db, args),
    };

    if let Err(err) = result {
        eprintln!("{err}");
    }
}
