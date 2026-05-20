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
