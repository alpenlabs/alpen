//! Binary entry‑point for the offline Alpen database tool.
//! Parses CLI arguments with **Clap** and delegates to the `alpen_dbtool` lib.

mod cli;
mod cmd;
mod db;
mod output;
mod utils;

use std::{path::Path, process::exit};

use alpen_ee_database::EeProverDbSled;
use strata_db_store_sled::SledBackend;
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

    let Cli { datadir, cmd } = argh::from_env();

    // Each command opens exactly one sled — OL or EE — under `--datadir`.
    // Sled takes an exclusive lock on the directory, so opening eagerly
    // would block parallel dbtool invocations against the same datadir
    // and force the operator to point `-d` at a path the chosen command
    // doesn't even need.
    let result = match cmd {
        Command::GetOLState(args) => with_ol_db(&datadir, |db| get_ol_state(db, args)),
        Command::RevertOLState(args) => with_ol_db(&datadir, |db| revert_ol_state(db, args)),
        Command::GetOlBlock(args) => with_ol_db(&datadir, |db| get_ol_block(db, args)),
        Command::GetOlSummary(args) => with_ol_db(&datadir, |db| get_ol_summary(db, args)),
        Command::GetL1Block(args) => with_ol_db(&datadir, |db| get_l1_block(db, args)),
        Command::GetL1Summary(args) => with_ol_db(&datadir, |db| get_l1_summary(db, args)),
        Command::GetWriterSummary(args) => with_ol_db(&datadir, |db| get_writer_summary(db, args)),
        Command::GetWriterPayload(args) => with_ol_db(&datadir, |db| get_writer_payload(db, args)),
        Command::GetCheckpoint(args) => with_ol_db(&datadir, |db| get_checkpoint(db, args)),
        Command::GetCheckpointsSummary(args) => {
            with_ol_db(&datadir, |db| get_checkpoints_summary(db, args))
        }
        Command::GetBroadcasterSummary(args) => with_ol_db(&datadir, |db| {
            get_broadcaster_summary(db.broadcast_db(), args)
        }),
        Command::GetBroadcasterTx(args) => {
            with_ol_db(&datadir, |db| get_broadcaster_tx(db.broadcast_db(), args))
        }
        Command::GetEpochSummary(args) => with_ol_db(&datadir, |db| get_epoch_summary(db, args)),
        Command::GetSyncinfo(args) => with_ol_db(&datadir, |db| get_syncinfo(db, args)),
        Command::GetClientStateUpdate(args) => {
            with_ol_db(&datadir, |db| get_client_state_update(db, args))
        }
        Command::GetProverTask(args) => with_ol_db(&datadir, |db| get_prover_task(db, args)),
        Command::GetProverTasksSummary(args) => {
            with_ol_db(&datadir, |db| get_prover_tasks_summary(db, args))
        }
        Command::AbandonProverTask(args) => {
            with_ol_db(&datadir, |db| abandon_prover_task(db, args))
        }
        Command::AbandonProverTasks(args) => {
            with_ol_db(&datadir, |db| abandon_prover_tasks(db, args))
        }
        Command::ResetProverTask(args) => with_ol_db(&datadir, |db| reset_prover_task(db, args)),
        Command::DeleteProverTask(args) => with_ol_db(&datadir, |db| delete_prover_task(db, args)),
        Command::GetCheckpointProof(args) => {
            with_ol_db(&datadir, |db| get_checkpoint_proof(db, args))
        }
        Command::DeleteCheckpointProof(args) => {
            with_ol_db(&datadir, |db| delete_checkpoint_proof(db, args))
        }
        Command::BackfillCheckpointProofTask(args) => {
            with_ol_db(&datadir, |db| backfill_checkpoint_proof_task(db, args))
        }
        Command::BackfillProverTaskRaw(args) => {
            with_ol_db(&datadir, |db| backfill_prover_task_raw(db, args))
        }
        Command::EeGetProverTask(args) => with_ee_db(&datadir, |db| ee_get_prover_task(db, args)),
        Command::EeGetProverTasksSummary(args) => {
            with_ee_db(&datadir, |db| ee_get_prover_tasks_summary(db, args))
        }
        Command::EeAbandonProverTask(args) => {
            with_ee_db(&datadir, |db| ee_abandon_prover_task(db, args))
        }
        Command::EeAbandonProverTasks(args) => {
            with_ee_db(&datadir, |db| ee_abandon_prover_tasks(db, args))
        }
        Command::EeResetProverTask(args) => {
            with_ee_db(&datadir, |db| ee_reset_prover_task(db, args))
        }
        Command::EeDeleteProverTask(args) => {
            with_ee_db(&datadir, |db| ee_delete_prover_task(db, args))
        }
        Command::EeBackfillProverTaskRaw(args) => {
            with_ee_db(&datadir, |db| ee_backfill_prover_task_raw(db, args))
        }
        Command::EeGetChunkReceipt(args) => {
            with_ee_db(&datadir, |db| ee_get_chunk_receipt(db, args))
        }
        Command::EeDeleteChunkReceipt(args) => {
            with_ee_db(&datadir, |db| ee_delete_chunk_receipt(db, args))
        }
        Command::EeGetAcctProof(args) => with_ee_db(&datadir, |db| ee_get_acct_proof(db, args)),
        Command::EeDeleteAcctProof(args) => {
            with_ee_db(&datadir, |db| ee_delete_acct_proof(db, args))
        }
    };

    if let Err(e) = result {
        eprintln!("{e}");
        exit(1);
    }
}

/// Opens the OL sled at `datadir` and runs `f` against it.
fn with_ol_db<F, R>(datadir: &Path, f: F) -> R
where
    F: FnOnce(&SledBackend) -> R,
{
    let db = open_database(datadir).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    });
    f(db.as_ref())
}

/// Opens the EE prover sled at `datadir` and runs `f` against it.
fn with_ee_db<F, R>(datadir: &Path, f: F) -> R
where
    F: FnOnce(&EeProverDbSled) -> R,
{
    let db = open_ee_database(datadir).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    });
    f(db.as_ref())
}
