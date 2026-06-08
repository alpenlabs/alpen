//! Admin commands operating on the OL prover task store.
//!
//! These talk directly to [`strata_db_types::prover_task::ProverTaskDatabase`]
//! so they can manipulate records without going through the running
//! prover service — by design, the node must be offline.
//!
//! Every mutating verb follows the `revert-ol-state` UX: without
//! `-f/--force` the command is a dry run (prints what *would* happen
//! and a `Use --force to execute these changes.` hint); with `--force`
//! the mutation actually lands.

use argh::FromArgs;
use strata_checkpoint_types::CheckpointProofTask;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::{backend::DatabaseBackend, prover_task::ProverTaskDatabase};
use strata_identifiers::Epoch;
use strata_paas::{TaskRecordData, TaskStatus};

use crate::{
    cli::OutputFormat,
    cmd::{
        checkpoint::get_canonical_epoch_commitment_at,
        prover_task_common::{parse_task_key, print_force_hint, StatusFilter, ABANDONED_REASON},
    },
    output::{
        output,
        prover_task::{ProverTaskInfo, ProverTasksSummaryInfo},
    },
};

/// Fetch a single prover task record by its hex-encoded key.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-prover-task")]
pub(crate) struct GetProverTaskArgs {
    /// hex-encoded task key (as stored by `ProverTaskDatabase`)
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Summarize prover tasks by status, with a bounded slice of entries.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-prover-tasks-summary")]
pub(crate) struct GetProverTasksSummaryArgs {
    /// status filter: all (default), pending, proving, completed,
    /// transient-failure, permanent-failure, unfinished, terminal
    #[argh(option, default = "StatusFilter::All")]
    pub(crate) status: StatusFilter,

    /// max number of matching entries to include in the output (default 20)
    #[argh(option, default = "20")]
    pub(crate) limit: usize,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Fetch a single prover task record by hex-encoded key.
pub(crate) fn get_prover_task(
    db: &impl DatabaseBackend,
    args: GetProverTaskArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let record = db
        .prover_task_db()
        .get_task(key.clone())
        .internal_error("Failed to read prover task record")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No prover task found for key".to_string(),
                Box::new(args.key_hex.clone()),
            )
        })?;

    let info = ProverTaskInfo::from_record(&key, &record);
    output(&info, args.output_format)
}

/// Summarize prover task store contents.
pub(crate) fn get_prover_tasks_summary(
    db: &impl DatabaseBackend,
    args: GetProverTasksSummaryArgs,
) -> Result<(), DisplayedError> {
    let task_db = db.prover_task_db();

    let all = task_db
        .list_all_tasks()
        .internal_error("Failed to list prover tasks")?;

    let mut pending = 0usize;
    let mut proving = 0usize;
    let mut completed = 0usize;
    let mut blocked = 0usize;
    let mut transient_failure = 0usize;
    let mut permanent_failure = 0usize;
    let mut matched = 0usize;
    let mut entries: Vec<ProverTaskInfo> = Vec::new();

    for (key, record) in &all {
        match record.status() {
            TaskStatus::Pending => pending += 1,
            TaskStatus::Proving { .. } => proving += 1,
            TaskStatus::Completed => completed += 1,
            TaskStatus::Blocked { .. } => blocked += 1,
            TaskStatus::TransientFailure { .. } => transient_failure += 1,
            TaskStatus::PermanentFailure { .. } => permanent_failure += 1,
        }
        if args.status.matches(record.status()) {
            matched += 1;
            if entries.len() < args.limit {
                entries.push(ProverTaskInfo::from_record(key, record));
            }
        }
    }

    let summary = ProverTasksSummaryInfo {
        total: all.len(),
        pending,
        proving,
        completed,
        blocked,
        transient_failure,
        permanent_failure,
        matched,
        returned: entries.len(),
        entries,
    };

    output(&summary, args.output_format)
}

/// Mark a single prover task as `PermanentFailure { error: "abandoned via dbtool" }`.
///
/// Leaves the record in the DB for audit; recovery will not respawn it.
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "abandon-prover-task")]
pub(crate) struct AbandonProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Bulk-abandon every Pending/Proving prover task.
///
/// Use case: after a crash or operator-induced restart, prevent stuck
/// in-progress tasks from being respawned by the recovery scanner.
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "abandon-prover-tasks")]
pub(crate) struct AbandonProverTasksArgs {
    /// only consider Pending/Proving tasks (currently the only supported
    /// selector — kept explicit so future selectors can be added)
    #[argh(switch)]
    pub(crate) all_unfinished: bool,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Reset a prover task to `Pending` and clear its retry-after timestamp.
///
/// Use case: force a fresh prove attempt (drops accumulated retry count).
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "reset-prover-task")]
pub(crate) struct ResetProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Hard-delete a prover task record.
///
/// Prefer `abandon-prover-task` unless you really want the row gone.
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "delete-prover-task")]
pub(crate) struct DeleteProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Abandon a single task by flipping its status to `PermanentFailure`.
pub(crate) fn abandon_prover_task(
    db: &impl DatabaseBackend,
    args: AbandonProverTaskArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let task_db = db.prover_task_db();

    let mut record = task_db
        .get_task(key.clone())
        .internal_error("Failed to read prover task record")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No prover task found for key".to_string(),
                Box::new(args.key_hex.clone()),
            )
        })?;

    if record.status().is_terminal() {
        return Err(DisplayedError::UserError(
            "Task is already in a terminal state".to_string(),
            Box::new(args.key_hex),
        ));
    }

    if !args.force {
        println!("would abandon: {}", args.key_hex);
        print_force_hint();
        return Ok(());
    }

    record.set_status(TaskStatus::PermanentFailure {
        error: ABANDONED_REASON.to_string(),
    });
    task_db
        .put_task(key, record)
        .internal_error("Failed to persist abandoned task")?;

    println!("abandoned: {}", args.key_hex);
    Ok(())
}

/// Bulk-abandon every Pending/Proving task.
pub(crate) fn abandon_prover_tasks(
    db: &impl DatabaseBackend,
    args: AbandonProverTasksArgs,
) -> Result<(), DisplayedError> {
    if !args.all_unfinished {
        return Err(DisplayedError::UserError(
            "--all-unfinished is the only currently supported selector".to_string(),
            Box::new(()),
        ));
    }

    let task_db = db.prover_task_db();
    let unfinished = task_db
        .list_unfinished()
        .internal_error("Failed to list unfinished prover tasks")?;

    let mut abandoned = 0usize;
    for (key, mut record) in unfinished {
        let key_hex = hex::encode(&key);
        if args.force {
            record.set_status(TaskStatus::PermanentFailure {
                error: ABANDONED_REASON.to_string(),
            });
            task_db
                .put_task(key, record)
                .internal_error("Failed to persist abandoned task")?;
            println!("abandoned: {key_hex}");
        } else {
            println!("would abandon: {key_hex}");
        }
        abandoned += 1;
    }

    let verb = if args.force {
        "abandoned"
    } else {
        "would abandon"
    };
    println!("{verb} {abandoned} task(s)");
    if !args.force {
        print_force_hint();
    }
    Ok(())
}

/// Reset a task to `Pending` and clear its retry-after timestamp.
pub(crate) fn reset_prover_task(
    db: &impl DatabaseBackend,
    args: ResetProverTaskArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let task_db = db.prover_task_db();

    let mut record = task_db
        .get_task(key.clone())
        .internal_error("Failed to read prover task record")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No prover task found for key".to_string(),
                Box::new(args.key_hex.clone()),
            )
        })?;

    if !args.force {
        println!("would reset: {}", args.key_hex);
        print_force_hint();
        return Ok(());
    }

    record.set_status(TaskStatus::Pending);
    record.set_retry_after_secs(None);
    task_db
        .put_task(key, record)
        .internal_error("Failed to persist reset task")?;

    println!("reset: {}", args.key_hex);
    Ok(())
}

/// Hard-delete a task row.
pub(crate) fn delete_prover_task(
    db: &impl DatabaseBackend,
    args: DeleteProverTaskArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let task_db = db.prover_task_db();

    // Resolve existence up front so the dry run can surface a clear
    // error rather than silently "previewing" a no-op delete.
    let exists = task_db
        .get_task(key.clone())
        .internal_error("Failed to read prover task record")?
        .is_some();
    if !exists {
        return Err(DisplayedError::UserError(
            "No prover task found for key".to_string(),
            Box::new(args.key_hex),
        ));
    }

    if !args.force {
        println!("would delete: {}", args.key_hex);
        print_force_hint();
        return Ok(());
    }

    task_db
        .delete_task(key)
        .internal_error("Failed to delete prover task")?;

    println!("deleted: {}", args.key_hex);
    Ok(())
}

/// Queue a fresh `Pending` checkpoint-proof task for an epoch.
///
/// Resolves the canonical commitment at the epoch and constructs the
/// task key via [`CheckpointProofTask`] so the running node picks it up
/// on next startup recovery. Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "backfill-checkpoint-proof-task")]
pub(crate) struct BackfillCheckpointProofTaskArgs {
    /// checkpoint epoch
    #[argh(positional)]
    pub(crate) epoch: Epoch,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Insert a `Pending` task record under a raw hex-encoded key.
///
/// Escape hatch for proof kinds without a typed helper. Caller is
/// responsible for matching the key format the host expects.
/// Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "backfill-prover-task-raw")]
pub(crate) struct BackfillProverTaskRawArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
}

/// Insert a Pending checkpoint-proof task for the canonical commitment at
/// the given epoch.
pub(crate) fn backfill_checkpoint_proof_task(
    db: &impl DatabaseBackend,
    args: BackfillCheckpointProofTaskArgs,
) -> Result<(), DisplayedError> {
    let commitment = get_canonical_epoch_commitment_at(db, args.epoch)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No canonical checkpoint commitment at epoch".to_string(),
            Box::new(args.epoch),
        )
    })?;
    let task = CheckpointProofTask(commitment);
    let key = task.to_key_bytes();
    let key_hex = hex::encode(&key);

    if !args.force {
        println!(
            "would backfill checkpoint proof task: {key_hex} (epoch {})",
            args.epoch
        );
        print_force_hint();
        return Ok(());
    }

    let record = TaskRecordData::new(TaskStatus::Pending);
    db.prover_task_db()
        .insert_task(key, record)
        .internal_error("Failed to insert prover task")?;

    println!(
        "backfilled checkpoint proof task: {key_hex} (epoch {})",
        args.epoch
    );
    Ok(())
}

/// Insert a Pending task under a caller-provided raw key.
pub(crate) fn backfill_prover_task_raw(
    db: &impl DatabaseBackend,
    args: BackfillProverTaskRawArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;

    if !args.force {
        println!("would backfill prover task: {}", args.key_hex);
        print_force_hint();
        return Ok(());
    }

    let record = TaskRecordData::new(TaskStatus::Pending);
    db.prover_task_db()
        .insert_task(key, record)
        .internal_error("Failed to insert prover task")?;

    println!("backfilled prover task: {}", args.key_hex);
    Ok(())
}
