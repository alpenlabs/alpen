//! Admin commands operating on the EE prover task store.
//!
//! These talk directly to [`alpen_ee_database::EeProverDbSled`]'s
//! [`strata_db_types::traits::ProverTaskDatabase`] impl. Same DB contract
//! as the OL surface, but the underlying store lives in a separate sled
//! instance under the alpen-client datadir (`<ee-datadir>/sled`), so
//! mutations here can't race with OL writers.
//!
//! Chunk and acct tasks share one tree, disambiguated by a single-byte
//! kind tag at the start of the key (`b'c'` / `b'a'`). The `--kind`
//! filter on the summary and bulk-abandon commands selects on that tag.

use std::{fmt, str::FromStr};

use alpen_ee_database::EeProverDbSled;
use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::ProverTaskDatabase;
use strata_paas::{TaskRecordData, TaskStatus};

use crate::{
    cli::OutputFormat,
    cmd::prover_task_common::{parse_task_key, require_confirm, StatusFilter, ABANDONED_REASON},
    output::{
        output,
        prover_task::{ProverTaskInfo, ProverTasksSummaryInfo},
    },
};

/// EE task kind filter. Matches on the kind tag carried by the key's
/// first byte — the same convention used by the alpen-client's prover
/// builders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KindFilter {
    All,
    Chunk,
    Acct,
}

impl KindFilter {
    fn matches(&self, key: &[u8]) -> bool {
        match self {
            Self::All => true,
            Self::Chunk => key.first().copied() == Some(b'c'),
            Self::Acct => key.first().copied() == Some(b'a'),
        }
    }
}

#[derive(Debug)]
pub(crate) struct UnsupportedKindFilter;

impl fmt::Display for UnsupportedKindFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "must be one of: all, chunk, acct")
    }
}

impl FromStr for KindFilter {
    type Err = UnsupportedKindFilter;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "all" => Ok(Self::All),
            "chunk" => Ok(Self::Chunk),
            "acct" => Ok(Self::Acct),
            _ => Err(UnsupportedKindFilter),
        }
    }
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-get-prover-task")]
/// Fetch a single EE prover task record by its hex-encoded key.
pub(crate) struct EeGetProverTaskArgs {
    /// hex-encoded task key (as stored by `EeProverDbSled`)
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-get-prover-tasks-summary")]
/// Summarize EE prover tasks by status and kind, with a bounded slice
/// of entries.
pub(crate) struct EeGetProverTasksSummaryArgs {
    /// status filter: all (default), pending, proving, completed,
    /// transient-failure, permanent-failure, unfinished, terminal
    #[argh(option, default = "StatusFilter::All")]
    pub(crate) status: StatusFilter,

    /// kind filter: all (default), chunk, acct
    #[argh(option, default = "KindFilter::All")]
    pub(crate) kind: KindFilter,

    /// max number of matching entries to include in the output (default 20)
    #[argh(option, default = "20")]
    pub(crate) limit: usize,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-abandon-prover-task")]
/// Mark a single EE prover task as `PermanentFailure { error: "abandoned via dbtool" }`.
///
/// Leaves the record in the DB for audit; recovery will not respawn it.
pub(crate) struct EeAbandonProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// confirm the mutation (required — the command is a no-op without it)
    #[argh(switch)]
    pub(crate) confirm: bool,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-abandon-prover-tasks")]
/// Bulk-abandon every Pending/Proving EE prover task, optionally
/// restricted by kind.
pub(crate) struct EeAbandonProverTasksArgs {
    /// only consider Pending/Proving tasks (currently the only supported
    /// selector — kept explicit so future selectors can be added)
    #[argh(switch)]
    pub(crate) all_unfinished: bool,

    /// kind filter: all (default), chunk, acct
    #[argh(option, default = "KindFilter::All")]
    pub(crate) kind: KindFilter,

    /// confirm the mutation
    #[argh(switch)]
    pub(crate) confirm: bool,

    /// preview the change set without writing
    #[argh(switch)]
    pub(crate) dry_run: bool,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-reset-prover-task")]
/// Reset an EE prover task to `Pending` and clear its retry-after timestamp.
pub(crate) struct EeResetProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// confirm the mutation
    #[argh(switch)]
    pub(crate) confirm: bool,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-delete-prover-task")]
/// Hard-delete an EE prover task record.
///
/// Prefer `ee-abandon-prover-task` unless you really want the row gone.
pub(crate) struct EeDeleteProverTaskArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// confirm the deletion
    #[argh(switch)]
    pub(crate) confirm: bool,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-backfill-prover-task-raw")]
/// Insert a `Pending` EE task record under a raw hex-encoded key.
///
/// EE task keys are produced by the chunk/acct spec encodings — they're
/// not easily reconstructible offline, so this raw escape hatch is the
/// only supported backfill path.
pub(crate) struct EeBackfillProverTaskRawArgs {
    /// hex-encoded task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// confirm the insertion
    #[argh(switch)]
    pub(crate) confirm: bool,
}

pub(crate) fn ee_get_prover_task(
    db: &EeProverDbSled,
    args: EeGetProverTaskArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let record = db
        .get_task(key.clone())
        .internal_error("Failed to read EE prover task record")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No EE prover task found for key".to_string(),
                Box::new(args.key_hex.clone()),
            )
        })?;

    let info = ProverTaskInfo::from_ee_record(&key, &record);
    output(&info, args.output_format)
}

pub(crate) fn ee_get_prover_tasks_summary(
    db: &EeProverDbSled,
    args: EeGetProverTasksSummaryArgs,
) -> Result<(), DisplayedError> {
    let all = db
        .list_all_tasks()
        .internal_error("Failed to list EE prover tasks")?;

    let mut pending = 0usize;
    let mut proving = 0usize;
    let mut completed = 0usize;
    let mut transient_failure = 0usize;
    let mut permanent_failure = 0usize;
    let mut matched = 0usize;
    let mut entries: Vec<ProverTaskInfo> = Vec::new();

    for (key, record) in &all {
        // The aggregate counters always reflect the full set — kind +
        // status filters only affect what lands in `matched` / `entries`,
        // so an operator can still see how many rows live in the store.
        match record.status() {
            TaskStatus::Pending => pending += 1,
            TaskStatus::Proving { .. } => proving += 1,
            TaskStatus::Completed => completed += 1,
            TaskStatus::TransientFailure { .. } => transient_failure += 1,
            TaskStatus::PermanentFailure { .. } => permanent_failure += 1,
        }
        if args.status.matches(record.status()) && args.kind.matches(key) {
            matched += 1;
            if entries.len() < args.limit {
                entries.push(ProverTaskInfo::from_ee_record(key, record));
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

pub(crate) fn ee_abandon_prover_task(
    db: &EeProverDbSled,
    args: EeAbandonProverTaskArgs,
) -> Result<(), DisplayedError> {
    require_confirm(args.confirm, "abandon an EE prover task")?;
    let key = parse_task_key(&args.key_hex)?;

    let mut record = db
        .get_task(key.clone())
        .internal_error("Failed to read EE prover task record")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No EE prover task found for key".to_string(),
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
    db.put_task(key, record)
        .internal_error("Failed to persist abandoned EE task")?;

    println!("abandoned: {}", args.key_hex);
    Ok(())
}

pub(crate) fn ee_abandon_prover_tasks(
    db: &EeProverDbSled,
    args: EeAbandonProverTasksArgs,
) -> Result<(), DisplayedError> {
    if !args.all_unfinished {
        return Err(DisplayedError::UserError(
            "--all-unfinished is the only currently supported selector".to_string(),
            Box::new(()),
        ));
    }
    if !args.dry_run {
        require_confirm(args.confirm, "bulk-abandon EE prover tasks")?;
    }

    let unfinished = db
        .list_unfinished()
        .internal_error("Failed to list unfinished EE prover tasks")?;

    let mut abandoned = 0usize;
    for (key, mut record) in unfinished {
        if !args.kind.matches(&key) {
            continue;
        }
        let key_hex = hex::encode(&key);
        if args.dry_run {
            println!("would abandon: {key_hex}");
        } else {
            record.set_status(TaskStatus::PermanentFailure {
                error: ABANDONED_REASON.to_string(),
            });
            db.put_task(key, record)
                .internal_error("Failed to persist abandoned EE task")?;
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

pub(crate) fn ee_reset_prover_task(
    db: &EeProverDbSled,
    args: EeResetProverTaskArgs,
) -> Result<(), DisplayedError> {
    require_confirm(args.confirm, "reset an EE prover task")?;
    let key = parse_task_key(&args.key_hex)?;

    let mut record = db
        .get_task(key.clone())
        .internal_error("Failed to read EE prover task record")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No EE prover task found for key".to_string(),
                Box::new(args.key_hex.clone()),
            )
        })?;

    record.set_status(TaskStatus::Pending);
    record.set_retry_after_secs(None);
    db.put_task(key, record)
        .internal_error("Failed to persist reset EE task")?;

    println!("reset: {}", args.key_hex);
    Ok(())
}

pub(crate) fn ee_delete_prover_task(
    db: &EeProverDbSled,
    args: EeDeleteProverTaskArgs,
) -> Result<(), DisplayedError> {
    require_confirm(args.confirm, "delete an EE prover task")?;
    let key = parse_task_key(&args.key_hex)?;

    let existed = db
        .delete_task(key)
        .internal_error("Failed to delete EE prover task")?;
    if !existed {
        return Err(DisplayedError::UserError(
            "No EE prover task found for key".to_string(),
            Box::new(args.key_hex),
        ));
    }

    println!("deleted: {}", args.key_hex);
    Ok(())
}

pub(crate) fn ee_backfill_prover_task_raw(
    db: &EeProverDbSled,
    args: EeBackfillProverTaskRawArgs,
) -> Result<(), DisplayedError> {
    require_confirm(args.confirm, "backfill an EE prover task")?;
    let key = parse_task_key(&args.key_hex)?;
    let record = TaskRecordData::new(TaskStatus::Pending);

    db.insert_task(key, record)
        .internal_error("Failed to insert EE prover task")?;

    println!("backfilled EE prover task: {}", args.key_hex);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_filter_matches_chunk_acct_and_all() {
        assert!(KindFilter::All.matches(b"anything"));
        assert!(KindFilter::All.matches(&[]));

        assert!(KindFilter::Chunk.matches(b"c-foo"));
        assert!(!KindFilter::Chunk.matches(b"a-foo"));
        assert!(!KindFilter::Chunk.matches(&[]));

        assert!(KindFilter::Acct.matches(b"a-foo"));
        assert!(!KindFilter::Acct.matches(b"c-foo"));
        assert!(!KindFilter::Acct.matches(&[]));
    }

    #[test]
    fn kind_filter_from_str_accepts_three_canonical_values() {
        assert_eq!("all".parse::<KindFilter>().unwrap(), KindFilter::All);
        assert_eq!("CHUNK".parse::<KindFilter>().unwrap(), KindFilter::Chunk);
        assert_eq!("acct".parse::<KindFilter>().unwrap(), KindFilter::Acct);
        assert!("bogus".parse::<KindFilter>().is_err());
    }
}
