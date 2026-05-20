//! Admin commands operating on the OL prover task store.
//!
//! These talk directly to [`strata_db_types::traits::ProverTaskDatabase`]
//! so they can manipulate records without going through the running
//! prover service — by design, the node must be offline.

use std::{fmt, str::FromStr};

use argh::FromArgs;
use strata_checkpoint_types::CheckpointProofTask;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{DatabaseBackend, ProverTaskDatabase};
use strata_identifiers::Epoch;
use strata_paas::{TaskRecordData, TaskStatus};

use crate::{
    cli::OutputFormat,
    cmd::checkpoint::get_canonical_epoch_commitment_at,
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

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "backfill-checkpoint-proof-task")]
/// Queue a fresh `Pending` checkpoint-proof task for an epoch.
///
/// Resolves the canonical commitment at the epoch and constructs the
/// task key via [`CheckpointProofTask`] so the running node picks it up
/// on next startup recovery.
pub(crate) struct BackfillCheckpointProofTaskArgs {
    /// checkpoint epoch
    #[argh(positional)]
    pub(crate) epoch: Epoch,

    /// confirm the insertion
    #[argh(switch)]
    pub(crate) confirm: bool,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "backfill-prover-task-raw")]
/// Insert a `Pending` task record under a raw hex-encoded key.
///
/// Escape hatch for proof kinds without a typed helper. Caller is
/// responsible for matching the key format the host expects.
pub(crate) struct BackfillProverTaskRawArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// confirm the insertion
    #[argh(switch)]
    pub(crate) confirm: bool,
}

/// Insert a Pending checkpoint-proof task for the canonical commitment at
/// the given epoch.
pub(crate) fn backfill_checkpoint_proof_task(
    db: &impl DatabaseBackend,
    args: BackfillCheckpointProofTaskArgs,
) -> Result<(), DisplayedError> {
    require_confirm(args.confirm, "backfill a checkpoint-proof task")?;

    let commitment = get_canonical_epoch_commitment_at(db, args.epoch)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No canonical checkpoint commitment at epoch".to_string(),
            Box::new(args.epoch),
        )
    })?;
    let task = CheckpointProofTask(commitment);
    let key = task.to_key_bytes();
    let key_hex = hex::encode(&key);
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
    require_confirm(args.confirm, "backfill a prover task")?;

    let key = parse_task_key(&args.key_hex)?;
    let record = TaskRecordData::new(TaskStatus::Pending);

    db.prover_task_db()
        .insert_task(key, record)
        .internal_error("Failed to insert prover task")?;

    println!("backfilled prover task: {}", args.key_hex);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_filter_matches_each_variant() {
        let pending = TaskStatus::Pending;
        let proving = TaskStatus::Proving { retry_count: 2 };
        let completed = TaskStatus::Completed;
        let transient = TaskStatus::TransientFailure {
            retry_count: 1,
            error: "x".into(),
        };
        let permanent = TaskStatus::PermanentFailure { error: "y".into() };

        assert!(StatusFilter::All.matches(&pending));
        assert!(StatusFilter::All.matches(&permanent));

        assert!(StatusFilter::Pending.matches(&pending));
        assert!(!StatusFilter::Pending.matches(&proving));

        assert!(StatusFilter::Proving.matches(&proving));
        assert!(!StatusFilter::Proving.matches(&pending));

        assert!(StatusFilter::Completed.matches(&completed));
        assert!(!StatusFilter::Completed.matches(&pending));

        assert!(StatusFilter::TransientFailure.matches(&transient));
        assert!(!StatusFilter::TransientFailure.matches(&permanent));

        assert!(StatusFilter::PermanentFailure.matches(&permanent));
        assert!(!StatusFilter::PermanentFailure.matches(&transient));

        // Unfinished == Pending | Proving.
        assert!(StatusFilter::Unfinished.matches(&pending));
        assert!(StatusFilter::Unfinished.matches(&proving));
        assert!(!StatusFilter::Unfinished.matches(&completed));
        assert!(!StatusFilter::Unfinished.matches(&transient));

        // Terminal == Completed | PermanentFailure.
        assert!(StatusFilter::Terminal.matches(&completed));
        assert!(StatusFilter::Terminal.matches(&permanent));
        assert!(!StatusFilter::Terminal.matches(&pending));
        assert!(!StatusFilter::Terminal.matches(&transient));
    }

    #[test]
    fn status_filter_from_str_accepts_canonical_and_aliases() {
        assert_eq!("all".parse::<StatusFilter>().unwrap(), StatusFilter::All);
        assert_eq!(
            "PENDING".parse::<StatusFilter>().unwrap(),
            StatusFilter::Pending
        );
        // Both hyphenated and snake-cased forms are accepted.
        assert_eq!(
            "transient-failure".parse::<StatusFilter>().unwrap(),
            StatusFilter::TransientFailure
        );
        assert_eq!(
            "transient_failure".parse::<StatusFilter>().unwrap(),
            StatusFilter::TransientFailure
        );
        assert_eq!(
            "permanent-failure".parse::<StatusFilter>().unwrap(),
            StatusFilter::PermanentFailure
        );
        assert_eq!(
            "unfinished".parse::<StatusFilter>().unwrap(),
            StatusFilter::Unfinished
        );
        assert_eq!(
            "terminal".parse::<StatusFilter>().unwrap(),
            StatusFilter::Terminal
        );

        assert!("bogus".parse::<StatusFilter>().is_err());
    }

    #[test]
    fn parse_task_key_accepts_hex_with_and_without_prefix() {
        assert_eq!(
            parse_task_key("deadbeef").unwrap(),
            vec![0xde, 0xad, 0xbe, 0xef]
        );
        assert_eq!(
            parse_task_key("0xdeadbeef").unwrap(),
            vec![0xde, 0xad, 0xbe, 0xef]
        );
        assert_eq!(parse_task_key("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn parse_task_key_rejects_invalid_hex() {
        assert!(parse_task_key("not-hex").is_err());
        // Odd length is invalid for hex.
        assert!(parse_task_key("abc").is_err());
    }
}
