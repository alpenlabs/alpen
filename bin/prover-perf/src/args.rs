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
    /// e.g. `--programs evm-ee-stf,cl-stf` or `--programs evm-ee-stf --programs cl-stf`
    #[argh(option)]
    pub programs: Vec<String>,
}

/// Parses program strings into [`GuestProgram`] variants.
///
/// Supports both comma-separated values and repeated options:
/// - `--programs evm-ee-stf,cl-stf`
/// - `--programs evm-ee-stf --programs cl-stf`
pub fn parse_programs(raw: &[String]) -> Result<Vec<GuestProgram>, String> {
    raw.iter()
        .flat_map(|s| s.split(','))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<GuestProgram>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_programs_comma_separated() {
        let input = vec!["evm-ee-stf,cl-stf,checkpoint".to_string()];
        let result = parse_programs(&input).unwrap();
        assert_eq!(result.len(), 3);
        assert!(matches!(result[0], GuestProgram::EvmEeStf));
        assert!(matches!(result[1], GuestProgram::ClStf));
        assert!(matches!(result[2], GuestProgram::Checkpoint));
    }

    #[test]
    fn test_parse_programs_repeated_options() {
        let input = vec![
            "evm-ee-stf".to_string(),
            "cl-stf".to_string(),
            "checkpoint".to_string(),
        ];
        let result = parse_programs(&input).unwrap();
        assert_eq!(result.len(), 3);
        assert!(matches!(result[0], GuestProgram::EvmEeStf));
        assert!(matches!(result[1], GuestProgram::ClStf));
        assert!(matches!(result[2], GuestProgram::Checkpoint));
    }

    #[test]
    fn test_parse_programs_mixed() {
        let input = vec!["evm-ee-stf,cl-stf".to_string(), "checkpoint".to_string()];
        let result = parse_programs(&input).unwrap();
        assert_eq!(result.len(), 3);
        assert!(matches!(result[0], GuestProgram::EvmEeStf));
        assert!(matches!(result[1], GuestProgram::ClStf));
        assert!(matches!(result[2], GuestProgram::Checkpoint));
    }

    #[test]
    fn test_parse_programs_with_whitespace() {
        let input = vec!["evm-ee-stf , cl-stf".to_string()];
        let result = parse_programs(&input).unwrap();
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], GuestProgram::EvmEeStf));
        assert!(matches!(result[1], GuestProgram::ClStf));
    }

    #[test]
    fn test_parse_programs_empty_input() {
        let input: Vec<String> = vec![];
        let result = parse_programs(&input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_programs_empty_strings() {
        let input = vec!["".to_string(), "  ".to_string()];
        let result = parse_programs(&input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_programs_invalid() {
        let input = vec!["invalid-program".to_string()];
        let result = parse_programs(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown program"));
    }

    #[test]
    fn test_parse_programs_partial_invalid() {
        let input = vec!["evm-ee-stf,invalid".to_string()];
        let result = parse_programs(&input);
        assert!(result.is_err());
    }
}
