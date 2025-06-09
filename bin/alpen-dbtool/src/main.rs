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
        syncinfo::get_syncinfo, Cli, Command,
    },
    db::{open_database, DbType},
    errors::Result,
};

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // // Safety: ensure node isn’t running by locking datadir.
    // let _guard = acquire_lock(&cli.datadir)?;

    let db = open_database(&cli.datadir, DbType::from_str(&cli.db_type).unwrap()).unwrap();

    match cli.cmd {
        Command::GetAlpenBlock(args) => get_alpen_block(db, args),
        Command::GetCheckpointData(args) => get_checkpoint_data(db, args),
        Command::GetSyncinfo(args) => get_syncinfo(db, args),
        Command::ResetChainstate(args) => reset_chainstate(db, args),
    }
}
