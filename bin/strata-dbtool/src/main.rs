//! Binary entry‑point for the offline Alpen database tool.
//! Parses CLI arguments with **Clap** and delegates to the `alpen_dbtool` lib.
mod cli;
mod cmd;
mod db;
mod utils;

use std::str::FromStr;

use crate::{
    cli::{Cli, Command},
    cmd::{
        chainstate::{get_chainstate, revert_chainstate},
        checkpoint::{get_checkpoint, get_checkpoints_summary, get_epoch_summary},
        client_state::get_client_state_update,
        l1::{get_l1_manifest, get_l1_summary},
        l2::{get_l2_block, get_l2_summary},
        sync_event::{get_sync_event, get_sync_events_summary},
        syncinfo::get_syncinfo,
    },
    db::{open_database, DbType},
};

fn main() {
    tracing_subscriber::fmt::init();

    let cli: Cli = argh::from_env();

    let db_type = DbType::from_str(&cli.db_type).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    let db = open_database(&cli.datadir, db_type).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    let result = match cli.cmd {
        Command::GetChainstate(args) => get_chainstate(&db, args),
        Command::RevertChainstate(args) => revert_chainstate(&db, args),
        Command::GetL2Block(args) => get_l2_block(&db, args),
        Command::GetL2Summary(args) => get_l2_summary(&db, args),
        Command::GetL1Manifest(args) => get_l1_manifest(&db, args),
        Command::GetL1Summary(args) => get_l1_summary(&db, args),
        Command::GetCheckpoint(args) => get_checkpoint(&db, args),
        Command::GetCheckpointsSummary(args) => get_checkpoints_summary(&db, args),
        Command::GetEpochSummary(args) => get_epoch_summary(&db, args),
        Command::GetSyncinfo(args) => get_syncinfo(&db, args),
        Command::GetSyncEvent(args) => get_sync_event(&db, args),
        Command::GetSyncEventsSummary(args) => get_sync_events_summary(&db, args),
        Command::GetClientStateUpdate(args) => get_client_state_update(&db, args),
    };

    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
