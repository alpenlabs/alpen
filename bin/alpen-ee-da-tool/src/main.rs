//! Command-line entry point for EE DA reconstruction and verification.

mod cli;
mod config;
mod l1;
mod ol;
mod output;
mod snapshot;

use std::process::ExitCode;

use alpen_ee_da_l1_extraction::reassemble_da_blobs;
use alpen_ee_da_state_replay::{
    replay_blobs_from_genesis, replay_blobs_from_snapshot, ReplayError, ReplaySummary,
};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_identifiers::Buf32;

use crate::{
    cli::{Cli, OutputFormat},
    config::VerifierConfig,
    l1::{collect_envelopes, create_ready_client},
    ol::fetch_manifest_expected_root,
    output::{output, ReplayStart, Report, ReportInput},
    snapshot::{build_export_snapshot, load_snapshot, write_snapshot},
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
    let scan_output = collect_envelopes(&client, &config, cli.start_height, cli.end_height).await?;
    let envelope_count = scan_output.envelopes.len() as u64;
    let blobs = reassemble_da_blobs(scan_output.envelopes)
        .internal_error("failed to reassemble DA blobs")?;
    let input_snapshot = cli.snapshot.as_deref().map(load_snapshot).transpose()?;
    let replay_start = if input_snapshot.is_some() {
        ReplayStart::Snapshot
    } else {
        ReplayStart::Genesis
    };
    let replay_summary = match &input_snapshot {
        Some(snapshot) => replay_blobs_from_snapshot(snapshot, &blobs),
        None => replay_blobs_from_genesis(&config.chain_spec, &blobs),
    }
    .map_err(map_replay_error_to_displayed)?;
    let expected_state_root =
        resolve_expected_state_root(&config, &replay_summary, cli.expected_root).await?;

    if let Some(path) = cli.export_snapshot.as_deref() {
        let exported_snapshot =
            build_export_snapshot(input_snapshot.as_ref(), &blobs, &replay_summary)?;
        write_snapshot(path, &exported_snapshot)?;
    }

    Ok(Report::new(ReportInput {
        scan_stats: scan_output.stats,
        envelope_count,
        blobs_reassembled: blobs.len() as u64,
        replay_summary,
        replay_start,
        expected_state_root,
    }))
}

async fn resolve_expected_state_root(
    config: &VerifierConfig,
    replay_summary: &ReplaySummary,
    manual_expected_root: Option<Buf32>,
) -> Result<Option<Buf32>, DisplayedError> {
    if manual_expected_root.is_some() {
        return Ok(manual_expected_root);
    }

    let (Some(ol_rpc_url), Some(ee_snark_account_id)) =
        (config.ol_rpc_url.as_deref(), config.ee_snark_account_id)
    else {
        if config.ol_rpc_url.is_none() && config.ee_snark_account_id.is_none() {
            return Ok(None);
        }

        return Err(DisplayedError::UserError(
            "incomplete expected-root comparison config".to_string(),
            Box::new(ExpectedRootComparisonConfigError),
        ));
    };

    let Some(applied) = replay_summary.applied() else {
        return Ok(None);
    };

    let expected_root = fetch_manifest_expected_root(
        ol_rpc_url,
        ee_snark_account_id,
        applied.last_update_seq_no(),
    )
    .await?;
    Ok(Some(expected_root))
}

#[derive(Debug, thiserror::Error)]
#[error("ol_rpc_url and ee_snark_account_id must be configured together")]
struct ExpectedRootComparisonConfigError;

fn map_replay_error_to_displayed(error: ReplayError) -> DisplayedError {
    match error {
        ReplayError::InvalidChainSpec { .. } => {
            DisplayedError::UserError("invalid chain specification".to_string(), Box::new(error))
        }
        ReplayError::NonGenesisStart { .. } => DisplayedError::UserError(
            "requested DA range does not start at genesis".to_string(),
            Box::new(error),
        ),
        ReplayError::SnapshotRootMismatch { .. }
        | ReplayError::SnapshotUpdateSeqNoMismatch { .. }
        | ReplayError::SnapshotBlockAnchorMismatch { .. }
        | ReplayError::InvalidSnapshotState { .. } => {
            DisplayedError::UserError("invalid replay snapshot".to_string(), Box::new(error))
        }
        ReplayError::UpdateSeqNoGap { .. }
        | ReplayError::DuplicateUpdateSeqNo { .. }
        | ReplayError::NonIncreasingBlockNumber { .. } => DisplayedError::UserError(
            "DA blob sequence is inconsistent".to_string(),
            Box::new(error),
        ),
        ReplayError::ApplyDiff { .. } => DisplayedError::InternalError(
            "state reconstruction failed while replaying DA blobs".to_string(),
            Box::new(error),
        ),
    }
}
