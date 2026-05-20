//! Admin commands operating on the OL prover task store.
//!
//! These talk directly to [`strata_db_types::traits::ProverTaskDatabase`]
//! so they can manipulate records without going through the running
//! prover service — by design, the node must be offline.

use std::{fmt, str::FromStr};

use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{DatabaseBackend, ProverTaskDatabase};
use strata_paas::TaskStatus;

use crate::{
    cli::OutputFormat,
    output::{
        output,
        prover_task::{ProverTaskInfo, ProverTasksSummaryInfo},
    },
};

/// Status filter accepted by the summary command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusFilter {
    All,
    Pending,
    Proving,
    Completed,
    TransientFailure,
    PermanentFailure,
    /// Pending or Proving — what the prover's startup recovery would respawn.
    Unfinished,
    /// Completed or PermanentFailure — won't be retried again.
    Terminal,
}

impl StatusFilter {
    fn matches(&self, status: &TaskStatus) -> bool {
        match self {
            Self::All => true,
            Self::Pending => matches!(status, TaskStatus::Pending),
            Self::Proving => matches!(status, TaskStatus::Proving { .. }),
            Self::Completed => matches!(status, TaskStatus::Completed),
            Self::TransientFailure => matches!(status, TaskStatus::TransientFailure { .. }),
            Self::PermanentFailure => matches!(status, TaskStatus::PermanentFailure { .. }),
            Self::Unfinished => status.is_unfinished(),
            Self::Terminal => status.is_terminal(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct UnsupportedStatusFilter;

impl fmt::Display for UnsupportedStatusFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "must be one of: all, pending, proving, completed, transient-failure, \
             permanent-failure, unfinished, terminal"
        )
    }
}

impl FromStr for StatusFilter {
    type Err = UnsupportedStatusFilter;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "all" => Ok(Self::All),
            "pending" => Ok(Self::Pending),
            "proving" => Ok(Self::Proving),
            "completed" => Ok(Self::Completed),
            "transient-failure" | "transient_failure" => Ok(Self::TransientFailure),
            "permanent-failure" | "permanent_failure" => Ok(Self::PermanentFailure),
            "unfinished" => Ok(Self::Unfinished),
            "terminal" => Ok(Self::Terminal),
            _ => Err(UnsupportedStatusFilter),
        }
    }
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-prover-task")]
/// Fetch a single prover task record by its hex-encoded key.
pub(crate) struct GetProverTaskArgs {
    /// hex-encoded task key (as stored by `ProverTaskDatabase`)
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-prover-tasks-summary")]
/// Summarize prover tasks by status, with a bounded slice of entries.
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

/// Parse a hex string into a task key, normalizing a `0x` prefix.
pub(crate) fn parse_task_key(hex_str: &str) -> Result<Vec<u8>, DisplayedError> {
    let trimmed = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    hex::decode(trimmed).map_err(|e| {
        DisplayedError::UserError("Invalid hex-encoded task key".to_string(), Box::new(e))
    })
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
    let mut transient_failure = 0usize;
    let mut permanent_failure = 0usize;
    let mut matched = 0usize;
    let mut entries: Vec<ProverTaskInfo> = Vec::new();

    for (key, record) in &all {
        match record.status() {
            TaskStatus::Pending => pending += 1,
            TaskStatus::Proving { .. } => proving += 1,
            TaskStatus::Completed => completed += 1,
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
        transient_failure,
        permanent_failure,
        matched,
        returned: entries.len(),
        entries,
    };

    output(&summary, args.output_format)
}

/// Error string written into `PermanentFailure` by the abandon commands.
const ABANDONED_REASON: &str = "abandoned via dbtool";

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "abandon-prover-task")]
/// Mark a single prover task as `PermanentFailure { error: "abandoned via dbtool" }`.
///
/// Leaves the record in the DB for audit; recovery will not respawn it.
pub(crate) struct AbandonProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// confirm the mutation (required — the command is a no-op without it)
    #[argh(switch)]
    pub(crate) confirm: bool,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "abandon-prover-tasks")]
/// Bulk-abandon every Pending/Proving prover task.
///
/// Use case: after a crash or operator-induced restart, prevent stuck
/// in-progress tasks from being respawned by the recovery scanner.
pub(crate) struct AbandonProverTasksArgs {
    /// only consider Pending/Proving tasks (currently the only supported
    /// selector — kept explicit so future selectors can be added)
    #[argh(switch)]
    pub(crate) all_unfinished: bool,

    /// confirm the mutation
    #[argh(switch)]
    pub(crate) confirm: bool,

    /// preview the change set without writing
    #[argh(switch)]
    pub(crate) dry_run: bool,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "reset-prover-task")]
/// Reset a prover task to `Pending` and clear its retry-after timestamp.
///
/// Use case: force a fresh prove attempt (drops accumulated retry count).
pub(crate) struct ResetProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// confirm the mutation
    #[argh(switch)]
    pub(crate) confirm: bool,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "delete-prover-task")]
/// Hard-delete a prover task record.
///
/// Prefer `abandon-prover-task` unless you really want the row gone.
pub(crate) struct DeleteProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// confirm the deletion
    #[argh(switch)]
    pub(crate) confirm: bool,
}

/// Common confirm-required guard.
fn require_confirm(confirm: bool, action: &str) -> Result<(), DisplayedError> {
    if confirm {
        Ok(())
    } else {
        Err(DisplayedError::UserError(
            format!("--confirm is required to {action}"),
            Box::new(()),
        ))
    }
}

/// Abandon a single task by flipping its status to `PermanentFailure`.
pub(crate) fn abandon_prover_task(
    db: &impl DatabaseBackend,
    args: AbandonProverTaskArgs,
) -> Result<(), DisplayedError> {
    require_confirm(args.confirm, "abandon a prover task")?;
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
    if !args.dry_run {
        require_confirm(args.confirm, "bulk-abandon prover tasks")?;
    }

    let task_db = db.prover_task_db();
    let unfinished = task_db
        .list_unfinished()
        .internal_error("Failed to list unfinished prover tasks")?;

    let mut abandoned = 0usize;
    for (key, mut record) in unfinished {
        let key_hex = hex::encode(&key);
        if args.dry_run {
            println!("would abandon: {key_hex}");
        } else {
            record.set_status(TaskStatus::PermanentFailure {
                error: ABANDONED_REASON.to_string(),
            });
            task_db
                .put_task(key, record)
                .internal_error("Failed to persist abandoned task")?;
            println!("abandoned: {key_hex}");
        }
        abandoned += 1;
    }

    let verb = if args.dry_run {
        "would abandon"
    } else {
        "abandoned"
    };
    println!("{verb} {abandoned} task(s)");
    Ok(())
}

/// Reset a task to `Pending` and clear its retry-after timestamp.
pub(crate) fn reset_prover_task(
    db: &impl DatabaseBackend,
    args: ResetProverTaskArgs,
) -> Result<(), DisplayedError> {
    require_confirm(args.confirm, "reset a prover task")?;
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
    require_confirm(args.confirm, "delete a prover task")?;
    let key = parse_task_key(&args.key_hex)?;
    let task_db = db.prover_task_db();

    let existed = task_db
        .delete_task(key)
        .internal_error("Failed to delete prover task")?;
    if !existed {
        return Err(DisplayedError::UserError(
            "No prover task found for key".to_string(),
            Box::new(args.key_hex),
        ));
    }

    println!("deleted: {}", args.key_hex);
    Ok(())
}
