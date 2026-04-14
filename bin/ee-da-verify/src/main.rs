//! Reconstructs EE DA state from Bitcoin DA transactions and optionally
//! verifies it against an expected state root.

mod cli;
mod config;
mod output;

use std::process::ExitCode;

use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::{
    cli::{Cli, OutputFormat},
    config::VerifierConfig,
    output::{output, Report},
};

fn main() -> ExitCode {
    let cli: Cli = argh::from_env();
    let format = cli.output_format.unwrap_or(OutputFormat::Porcelain);
    match run(&cli).and_then(|report| output(&report, format)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<Report, DisplayedError> {
    let _config = VerifierConfig::load(&cli.config)
        .user_error(format!("failed to load {}", cli.config.display()))?;
    Ok(Report {})
}
