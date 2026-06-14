use argh::FromArgs;

use crate::programs::GuestProgram;

/// Evaluate the performance of SP1 on programs.
#[derive(Debug, Clone, FromArgs)]
pub struct EvalArgs {
    /// whether to post on github or run locally and only log the results
    #[argh(switch)]
    pub post_to_gh: bool,

    /// the GitHub token for authentication
    #[argh(option, default = "String::new()")]
    pub github_token: String,

    /// the GitHub PR number
    #[argh(option, default = "String::new()")]
    pub pr_number: String,

    /// the commit hash
    #[argh(option, default = "String::from(\"local_commit\")")]
    pub commit_hash: String,

    /// programs to run (comma-delimited and/or repeated),
    /// e.g. `--programs alpen-chunk,checkpoint` or `--programs alpen-chunk
    /// --programs checkpoint`
    #[argh(option)]
    pub programs: Vec<String>,

    /// generate full proofs instead of only executing the SP1 guests
    #[argh(switch)]
    pub prove: bool,
}

/// Parses program strings into [`GuestProgram`] variants.
///
/// Supports both comma-separated values and repeated options:
/// - `--programs alpen-chunk,checkpoint`
/// - `--programs alpen-chunk --programs checkpoint`
pub fn parse_programs(raw: &[String]) -> Result<Vec<GuestProgram>, String> {
    raw.iter()
        .flat_map(|s| s.split(','))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<GuestProgram>())
        .collect()
}
