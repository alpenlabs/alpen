//! Reconstructs EE DA state from Bitcoin DA transactions and optionally
//! verifies it against an expected state root.

mod cli;
mod config;
mod l1;
mod output;
mod replay;

use std::process::ExitCode;

use strata_cli_common::errors::{DisplayableError, DisplayedError};

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
    let client = l1::create_ready_client(&config).await?;
    let scan_output =
        l1::collect_envelopes(&client, &config, cli.start_height, cli.end_height).await?;
    let envelope_count = scan_output.envelopes.len() as u64;
    let blobs = ee_da_l1::reassemble_da_blobs(scan_output.envelopes)
        .internal_error("failed to reassemble DA blobs")?;
    let replay_summary = replay::replay_blobs(&config.chain_spec, &blobs)?;

    Ok(Report::new(
        scan_output.stats,
        envelope_count,
        blobs.len() as u64,
        replay_summary,
        cli.expected_root,
    ))
}
