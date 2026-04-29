use std::{
    fmt::{self, Display},
    path::PathBuf,
    str::FromStr,
};

use argh::FromArgs;
use strata_identifiers::{Buf32, L1Height};

/// Report output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Porcelain,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputFormatParseError;

impl Display for OutputFormatParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("must be 'porcelain' or 'json'")
    }
}

impl FromStr for OutputFormat {
    type Err = OutputFormatParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "porcelain" => Ok(Self::Porcelain),
            "json" => Ok(Self::Json),
            _ => Err(OutputFormatParseError),
        }
    }
}

/// Parses command-line flags for EE DA verification.
#[derive(Debug, FromArgs, PartialEq, Eq)]
#[argh(description = "verify EE DA state roots from Bitcoin data")]
pub(crate) struct Cli {
    /// verifier config path (TOML).
    #[argh(option)]
    pub(crate) config: PathBuf,

    /// inclusive Bitcoin start height.
    #[argh(option)]
    pub(crate) start_height: L1Height,

    /// inclusive Bitcoin end height.
    #[argh(option)]
    pub(crate) end_height: L1Height,

    /// optional expected EE state root for comparison.
    #[argh(option)]
    pub(crate) expected_root: Option<Buf32>,

    /// report output format (`porcelain` or `json`).
    #[argh(option)]
    pub(crate) output_format: Option<OutputFormat>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use argh::FromArgs;
    use strata_identifiers::Buf32;

    use super::{Cli, OutputFormat};

    fn parse_cli(args: &[&str]) -> Result<Cli, argh::EarlyExit> {
        Cli::from_args(&["ee-da-verify"], args)
    }

    #[test]
    fn cli_parses_required_args() {
        let cli = parse_cli(&[
            "--config",
            "/tmp/ee-da-verify.toml",
            "--start-height",
            "100",
            "--end-height",
            "200",
        ])
        .expect("required args must parse");

        assert_eq!(cli.config, Path::new("/tmp/ee-da-verify.toml"));
        assert_eq!(cli.start_height, 100);
        assert_eq!(cli.end_height, 200);
        assert_eq!(cli.expected_root, None);
        assert_eq!(cli.output_format, None);
    }

    #[test]
    fn cli_parses_optional_args() {
        let cli = parse_cli(&[
            "--config",
            "/tmp/ee-da-verify.toml",
            "--start-height",
            "100",
            "--end-height",
            "200",
            "--expected-root",
            "0x0101010101010101010101010101010101010101010101010101010101010101",
            "--output-format",
            "json",
        ])
        .expect("optional args must parse");

        assert_eq!(cli.expected_root, Some(Buf32::from([0x01u8; 32])));
        assert_eq!(cli.output_format, Some(OutputFormat::Json));
    }

    #[test]
    fn cli_rejects_unknown_output_format() {
        let err = parse_cli(&[
            "--config",
            "/tmp/ee-da-verify.toml",
            "--start-height",
            "100",
            "--end-height",
            "200",
            "--output-format",
            "default",
        ])
        .expect_err("unknown format must fail");

        assert!(err.output.contains("must be 'porcelain' or 'json'"));
    }

    #[test]
    fn cli_requires_config_path() {
        let err = parse_cli(&["--start-height", "1", "--end-height", "2"])
            .expect_err("missing --config must fail");
        assert!(err.output.contains("--config"));
    }
}
