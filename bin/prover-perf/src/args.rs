use clap::Parser;
use zkaleido_perf_report::GithubReportArgs;

use crate::programs::GuestProgram;

/// Evaluate the performance of SP1 on programs.
#[derive(Debug, Clone, Parser)]
pub struct EvalArgs {
    /// GitHub reporting options. The report is posted to the PR only when at least one of these is
    /// passed.
    #[command(flatten)]
    pub github: Option<GithubReportArgs>,

    /// Programs to run (comma-delimited and/or repeated),
    /// e.g. `--programs checkpoint`.
    #[arg(long)]
    pub programs: Vec<String>,
}

/// Parses program strings into [`GuestProgram`] variants.
///
/// Supports both comma-separated values and repeated options:
/// - `--programs checkpoint`
/// - `--programs checkpoint --programs checkpoint`
pub fn parse_programs(raw: &[String]) -> Result<Vec<GuestProgram>, String> {
    raw.iter()
        .flat_map(|s| s.split(','))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<GuestProgram>())
        .collect()
}
