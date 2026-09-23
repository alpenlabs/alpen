use argh::FromArgs;

/// Evaluate checkpoint guest execution performance on SP1.
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

    /// guest to benchmark (only checkpoint; retained for existing CI commands)
    #[argh(option, default = "String::from(\"checkpoint\")")]
    pub programs: String,
}
