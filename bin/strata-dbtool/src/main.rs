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
        chainstate::{get_chainstate, revert_chainstate},
        checkpoint::{get_checkpoint, get_checkpoints_summary, get_epoch_summary},
        client_state::get_client_state_update,
        l1::{get_l1_manifest, get_l1_summary},
        l2::{get_l2_block, get_l2_summary},
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
        Command::GetChainstate(args) => get_chainstate(db, args),
        Command::RevertChainstate(args) => revert_chainstate(db, args),
        Command::GetL2Block(args) => get_l2_block(db, args),
        Command::GetL2Summary(args) => get_l2_summary(db, args),
        Command::GetL1Manifest(args) => get_l1_manifest(db, args),
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
