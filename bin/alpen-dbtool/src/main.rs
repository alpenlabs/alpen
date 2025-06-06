//! Binary entry‑point for the offline Alpen database tool.
//! Parses CLI arguments with **Clap** and delegates to the `alpen_dbtool` lib.
mod backend;
mod cmd;
mod errors;

use std::str::FromStr;

use clap::Parser;

use crate::{
    backend::{open_database, DbType},
    cmd::{get_syncinfo::get_syncinfo, reset_chainstate::reset_chainstate, Cli, Command},
    errors::Result,
};

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // // Safety: ensure node isn’t running by locking datadir.
    // let _guard = acquire_lock(&cli.datadir)?;

    let db = open_database(&cli.datadir, DbType::from_str(&cli.db_type).unwrap()).unwrap();

    match cli.cmd {
        Command::GetSyncinfo(args) => get_syncinfo(db, args),
        Command::ResetChainstate(args) => reset_chainstate(db, args),
    }
}
