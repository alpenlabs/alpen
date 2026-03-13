//! Binary entry‑point for the offline Alpen database tool.
//! Parses CLI arguments with **Clap** and delegates to the `alpen_dbtool` lib.

mod cli;
mod cmd;
mod db;
mod output;
mod utils;

use std::process::exit;

use strata_db_types::traits::DatabaseBackend;
use tracing_subscriber::fmt::init;

use crate::{
    cli::{Cli, Command},
    cmd::{
        broadcaster::{get_broadcaster_summary, get_broadcaster_tx},
        checkpoint::{get_checkpoint, get_checkpoints_summary, get_epoch_summary},
        client_state::get_client_state_update,
        l1::{get_l1_block, get_l1_summary},
        ol::{get_ol_block, get_ol_summary},
        ol_state::{get_ol_state, revert_ol_state},
        syncinfo::get_syncinfo,
        writer::{get_writer_payload, get_writer_summary},
    },
    db::open_database,
};

fn main() {
    init();

    let cli: Cli = argh::from_env();

    let db = open_database(&cli.datadir).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    });
    let db = db.as_ref();

    let result = match cli.cmd {
        Command::GetOLState(args) => get_ol_state(db, args),
        Command::RevertOLState(args) => revert_ol_state(db, args),
        Command::GetOlBlock(args) => get_ol_block(db, args),
        Command::GetOlSummary(args) => get_ol_summary(db, args),
        Command::GetL1Block(args) => get_l1_block(db, args),
        Command::GetL1Summary(args) => get_l1_summary(db, args),
        Command::GetWriterSummary(args) => get_writer_summary(db, args),
        Command::GetWriterPayload(args) => get_writer_payload(db, args),
        Command::GetCheckpoint(args) => get_checkpoint(db, args),
        Command::GetCheckpointsSummary(args) => get_checkpoints_summary(db, args),
        Command::GetBroadcasterSummary(args) => get_broadcaster_summary(db.broadcast_db(), args),
        Command::GetBroadcasterTx(args) => get_broadcaster_tx(db.broadcast_db(), args),
        Command::GetEpochSummary(args) => get_epoch_summary(db, args),
        Command::GetSyncinfo(args) => get_syncinfo(db, args),
        Command::GetClientStateUpdate(args) => get_client_state_update(db, args),
    };

    if let Err(e) = result {
        eprintln!("{e}");
        exit(1);
    }
}
