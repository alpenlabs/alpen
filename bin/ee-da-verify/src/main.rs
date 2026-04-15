//! Reconstructs EE DA state from Bitcoin DA transactions and optionally
//! verifies it against an expected state root.

mod cli;
mod config;
mod da;
mod l1;
mod output;

#[cfg(test)]
mod test_utils;

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
    let scan_output = l1::collect_reveals(
        &client,
        cli.start_height,
        cli.end_height,
        config.magic_bytes,
    )
    .await?;
    let envelopes = da::segment_reveals(scan_output.ordered_reveals)
        .internal_error("failed to segment reveal chain")?;
    Ok(Report {
        fetched_block_count: scan_output.fetched_block_count,
        blocks_with_reveals: scan_output.blocks_with_reveals,
        envelope_count: envelopes.len() as u64,
    })
}
