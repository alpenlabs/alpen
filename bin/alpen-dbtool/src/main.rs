//! Binary entry‑point for the offline Alpen database tool.
//! Parses CLI arguments with **Clap** and delegates to the `alpen_dbtool` lib.
mod cli;
mod cmd;
mod db;
mod errors;

use std::str::FromStr;

use clap::Parser;

use crate::{
    cli::{Cli, Command},
    cmd::{
        chainstate::reset_chainstate, checkpoint::get_checkpoint_data, epoch::get_epoch_summary,
        l1_manifest::get_l1_manifest, l2_block::get_l2_block, l2_client::get_l2_client_state,
        syncinfo::get_syncinfo,
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
        Command::GetL1Manifest(args) => get_l1_manifest(db, args),
        Command::GetL2Block(args) => get_l2_block(db, args),
        Command::GetL2ClientState(args) => get_l2_client_state(db, args),
        Command::GetCheckpointData(args) => get_checkpoint_data(db, args),
        Command::GetEpochSummary(args) => get_epoch_summary(db, args),
        Command::GetSyncinfo(args) => get_syncinfo(db, args),
        Command::ResetChainstate(args) => reset_chainstate(db, args),
    };

    if let Err(err) = result {
        eprintln!("{err}");
    }
}
