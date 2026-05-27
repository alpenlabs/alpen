//! Shared helpers for prover-task admin commands.
//!
//! Both the OL ([`super::prover_task`]) and EE ([`super::ee_prover_task`])
//! task-store surfaces accept the same status filters, hex-key encoding,
//! `--confirm` guard, and abandon-reason string. Keeping these in one
//! place ensures the two surfaces stay in lockstep — operator workflows
//! migrate between them with no surprises.

use std::{fmt, str::FromStr};

use strata_cli_common::errors::DisplayedError;
use strata_paas::TaskStatus;

/// Status filter accepted by the summary commands.
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
    pub(crate) fn matches(&self, status: &TaskStatus) -> bool {
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

/// Error string written into `PermanentFailure` by the abandon commands.
///
/// Operator workflows grep on this exact phrase, so it must stay stable
/// across the OL and EE surfaces.
pub(crate) const ABANDONED_REASON: &str = "abandoned via dbtool";

/// Parse a hex string into a task key, normalizing a `0x` prefix.
pub(crate) fn parse_task_key(hex_str: &str) -> Result<Vec<u8>, DisplayedError> {
    let trimmed = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    hex::decode(trimmed).map_err(|e| {
        DisplayedError::UserError("Invalid hex-encoded task key".to_string(), Box::new(e))
    })
}

/// Standard tail line printed at the end of a dry-run, matching the
/// phrasing used by `revert-ol-state`.
pub(crate) fn print_force_hint() {
    println!();
    println!("Use --force to execute these changes.");
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

        assert!(StatusFilter::Unfinished.matches(&pending));
        assert!(StatusFilter::Unfinished.matches(&proving));
        assert!(!StatusFilter::Unfinished.matches(&completed));
        assert!(!StatusFilter::Unfinished.matches(&transient));

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
        assert!(parse_task_key("abc").is_err());
    }
}
