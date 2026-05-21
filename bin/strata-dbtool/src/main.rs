//! Binary entry‑point for the offline Alpen database tool.
//! Parses CLI arguments with **Clap** and delegates to the `alpen_dbtool` lib.

mod cli;
mod cmd;
mod db;
mod output;
mod utils;

use std::{path::Path, process::exit};

use alpen_ee_database::EeProverDbSled;
use strata_cli_common::errors::DisplayedError;
use strata_db_types::traits::DatabaseBackend;
use tracing_subscriber::fmt::init;

use crate::{
    cli::{Cli, Command},
    cmd::{
        broadcaster::{get_broadcaster_summary, get_broadcaster_tx},
        checkpoint::{get_checkpoint, get_checkpoints_summary, get_epoch_summary},
        checkpoint_proof::{delete_checkpoint_proof, get_checkpoint_proof},
        client_state::get_client_state_update,
        ee_prover_task::{
            ee_abandon_prover_task, ee_abandon_prover_tasks, ee_backfill_prover_task_raw,
            ee_delete_prover_task, ee_get_prover_task, ee_get_prover_tasks_summary,
            ee_reset_prover_task,
        },
        ee_receipts::{
            ee_delete_acct_proof, ee_delete_chunk_receipt, ee_get_acct_proof, ee_get_chunk_receipt,
        },
        l1::{get_l1_block, get_l1_summary},
        ol::{get_ol_block, get_ol_summary},
        ol_state::{get_ol_state, revert_ol_state},
        prover_task::{
            abandon_prover_task, abandon_prover_tasks, backfill_checkpoint_proof_task,
            backfill_prover_task_raw, delete_prover_task, get_prover_task,
            get_prover_tasks_summary, reset_prover_task,
        },
        syncinfo::get_syncinfo,
        writer::{get_writer_payload, get_writer_summary},
    },
    db::{open_database, open_ee_database},
};

fn main() {
    init();

    let cli: Cli = argh::from_env();

    let db = open_database(&cli.datadir).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    });
    let db = db.as_ref();

    // The EE DB is only opened when an `ee-*` command actually runs —
    // OL-only invocations must not require `--ee-datadir` to be set, and
    // sled itself takes an exclusive lock on the directory, so opening
    // eagerly would block parallel dbtool invocations on the same OL
    // datadir.
    let ee_datadir = cli.ee_datadir.as_deref();

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
        Command::GetProverTask(args) => get_prover_task(db, args),
        Command::GetProverTasksSummary(args) => get_prover_tasks_summary(db, args),
        Command::AbandonProverTask(args) => abandon_prover_task(db, args),
        Command::AbandonProverTasks(args) => abandon_prover_tasks(db, args),
        Command::ResetProverTask(args) => reset_prover_task(db, args),
        Command::DeleteProverTask(args) => delete_prover_task(db, args),
        Command::GetCheckpointProof(args) => get_checkpoint_proof(db, args),
        Command::DeleteCheckpointProof(args) => delete_checkpoint_proof(db, args),
        Command::BackfillCheckpointProofTask(args) => backfill_checkpoint_proof_task(db, args),
        Command::BackfillProverTaskRaw(args) => backfill_prover_task_raw(db, args),
        Command::EeGetProverTask(args) => with_ee_db(ee_datadir, |db| ee_get_prover_task(db, args)),
        Command::EeGetProverTasksSummary(args) => {
            with_ee_db(ee_datadir, |db| ee_get_prover_tasks_summary(db, args))
        }
        Command::EeAbandonProverTask(args) => {
            with_ee_db(ee_datadir, |db| ee_abandon_prover_task(db, args))
        }
        Command::EeAbandonProverTasks(args) => {
            with_ee_db(ee_datadir, |db| ee_abandon_prover_tasks(db, args))
        }
        Command::EeResetProverTask(args) => {
            with_ee_db(ee_datadir, |db| ee_reset_prover_task(db, args))
        }
        Command::EeDeleteProverTask(args) => {
            with_ee_db(ee_datadir, |db| ee_delete_prover_task(db, args))
        }
        Command::EeBackfillProverTaskRaw(args) => {
            with_ee_db(ee_datadir, |db| ee_backfill_prover_task_raw(db, args))
        }
        Command::EeGetChunkReceipt(args) => {
            with_ee_db(ee_datadir, |db| ee_get_chunk_receipt(db, args))
        }
        Command::EeDeleteChunkReceipt(args) => {
            with_ee_db(ee_datadir, |db| ee_delete_chunk_receipt(db, args))
        }
        Command::EeGetAcctProof(args) => with_ee_db(ee_datadir, |db| ee_get_acct_proof(db, args)),
        Command::EeDeleteAcctProof(args) => {
            with_ee_db(ee_datadir, |db| ee_delete_acct_proof(db, args))
        }
    };

    if let Err(e) = result {
        eprintln!("{e}");
        exit(1);
    }
}

/// Opens the EE prover db lazily and runs `f` against it.
///
/// Returns a user-facing error if `--ee-datadir` was not supplied, so
/// the operator sees the missing flag rather than a sled open failure.
fn with_ee_db<F>(ee_datadir: Option<&Path>, f: F) -> Result<(), DisplayedError>
where
    F: FnOnce(&EeProverDbSled) -> Result<(), DisplayedError>,
{
    let ee_datadir = ee_datadir.ok_or_else(|| {
        DisplayedError::UserError(
            "--ee-datadir is required for ee-* subcommands".to_string(),
            Box::new(()),
        )
    })?;
    let ee_db = open_ee_database(ee_datadir)?;
    f(ee_db.as_ref())
}
