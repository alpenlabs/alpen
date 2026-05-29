use argh::FromArgs;

use crate::programs::GuestProgram;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerfMode {
    Execute,
    Prove,
}

impl std::str::FromStr for PerfMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "execute" => Ok(Self::Execute),
            "prove" => Ok(Self::Prove),
            _ => Err(format!("unknown mode: {s}")),
        }
    }
}

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

    /// benchmark mode: `execute` for execution summary, `prove` for real proof generation
    #[argh(option, default = "String::from(\"execute\")")]
    pub mode: String,

    /// programs to run (comma-delimited and/or repeated),
    /// e.g. `--programs alpen-chunk,checkpoint` or `--programs alpen-chunk
    /// --programs checkpoint`
    #[argh(option)]
    pub programs: Vec<String>,
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

pub fn parse_mode(raw: &str) -> Result<PerfMode, String> {
    raw.parse()
}

pub fn validate_mode_programs(mode: PerfMode, programs: &[GuestProgram]) -> Result<(), String> {
    if mode == PerfMode::Prove
        && programs
            .iter()
            .any(|program| !matches!(program, GuestProgram::Checkpoint))
    {
        return Err(
            "prove mode currently supports only `checkpoint`; use `--programs checkpoint`"
                .to_string(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_accepts_known_values_case_insensitively() {
        assert_eq!(parse_mode("execute").unwrap(), PerfMode::Execute);
        assert_eq!(parse_mode("PrOvE").unwrap(), PerfMode::Prove);
    }

    #[test]
    fn parse_mode_rejects_unknown_values() {
        assert_eq!(parse_mode("gpu").unwrap_err(), "unknown mode: gpu");
    }

    #[test]
    fn validate_mode_programs_rejects_non_checkpoint_prove_targets() {
        let err = validate_mode_programs(PerfMode::Prove, &[GuestProgram::AlpenChunk]).unwrap_err();
        assert_eq!(
            err,
            "prove mode currently supports only `checkpoint`; use `--programs checkpoint`"
        );
    }

    #[test]
    fn validate_mode_programs_accepts_execute_mode_for_any_program() {
        validate_mode_programs(PerfMode::Execute, &[GuestProgram::AlpenAcct]).unwrap();
    }
}
