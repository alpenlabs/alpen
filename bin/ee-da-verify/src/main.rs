//! Reconstructs EE DA state from Bitcoin DA transactions and optionally
//! verifies it against an expected state root.

mod cli;
mod config;
mod l1;
mod output;

use std::process::ExitCode;

use strata_cli_common::errors::DisplayedError;

use crate::{
    cli::{Cli, OutputFormat},
    config::VerifierConfig,
    output::{output, Report},
};

#[tokio::main]
async fn main() -> ExitCode {
    let cli: Cli = argh::from_env();
    let format = cli.output_format.unwrap_or(OutputFormat::Porcelain);
    match run(&cli).await.and_then(|report| output(&report, format)) {
        Ok(()) => ExitCode::from(0),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: &Cli) -> Result<Report, DisplayedError> {
    let config = VerifierConfig::load(&cli.config)?;
    let client = l1::create_ready_client(&config).await?;
    let blocks_fetched = l1::count_blocks(&client, cli.start_height, cli.end_height).await?;
    Ok(Report { blocks_fetched })
}
