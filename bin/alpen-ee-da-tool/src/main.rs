//! Command-line entry point for EE DA reconstruction and verification.

mod cli;
mod config;
mod l1;
mod output;

use std::process::ExitCode;

use alpen_ee_da_l1_extraction::reassemble_da_blobs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::{
    cli::{Cli, OutputFormat},
    config::VerifierConfig,
    l1::{collect_envelopes, create_ready_client},
    output::{output, Report},
};

#[tokio::main]
async fn main() -> ExitCode {
    let cli: Cli = argh::from_env();
    let format = cli.output_format.unwrap_or(OutputFormat::Porcelain);
    match run(&cli).await.and_then(|report| output(&report, format)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: &Cli) -> Result<Report, DisplayedError> {
    let config = VerifierConfig::load(&cli.config)
        .user_error(format!("failed to load {}", cli.config.display()))?;
    let client = create_ready_client(&config).await?;
    let envelopes = collect_envelopes(&client, &config, cli.start_height, cli.end_height).await?;
    reassemble_da_blobs(envelopes).internal_error("failed to reassemble DA blobs")?;
    Ok(Report {})
}
