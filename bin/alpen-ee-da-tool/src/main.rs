//! Command-line entry point for EE DA reconstruction and verification.

mod cli;
mod config;
mod l1;
mod ol;
mod output;
mod replay;
mod snapshot;

use std::{fs, process::ExitCode};

use alpen_chainspec::{chain_value_parser, ee_genesis_block_info};
use alpen_ee_config::AlpenEeParams;
use alpen_ee_da_l1_extraction::reassemble_da_blobs;
use alpen_ee_da_state_replay::{
    replay_blobs_from_genesis, replay_blobs_from_snapshot, ReplayError, ReplaySummary,
};
use alpen_ee_genesis::build_genesis_ee_account_state;
use alpen_reth_statediff::StateReconstructor;
use ssz::{Decode, DecodeError};
use strata_acct_types::AccountId;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_ee_acct_types::EeAccountState;

use crate::{
    cli::{Cli, OutputFormat},
    config::VerifierConfig,
    l1::{collect_envelopes, create_ready_client},
    ol::{apply_published_update_and_get_inner_root, compute_account_inner_root},
    output::{output, InnerRootComparison, Report, ReportInput},
    replay::ReplayStart,
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
    let ee_params = load_ee_params(&config)?;
    validate_ee_params_genesis(&ee_params, &cli.custom_chain)?;
    let client = create_ready_client(&config).await?;
    let scan_output = collect_envelopes(&client, &config, cli.start_height, cli.end_height).await?;
    let envelope_count = scan_output.envelopes.len() as u64;
    let blobs = reassemble_da_blobs(scan_output.envelopes)
        .internal_error("failed to reassemble DA blobs")?;
    let input_snapshot = cli
        .snapshot_path
        .as_deref()
        .map(load_snapshot)
        .transpose()?;
    let replay_start = if input_snapshot.is_some() {
        ReplayStart::Snapshot
    } else {
        ReplayStart::Genesis
    };
    let replay_summary = match &input_snapshot {
        Some(snapshot) => replay_blobs_from_snapshot(snapshot, &blobs),
        None => {
            let reconstructor = create_genesis_reconstructor(&cli.custom_chain)?;
            replay_blobs_from_genesis(reconstructor, &blobs)
        }
    }
    .map_err(map_replay_error_to_displayed)?;
    let expected_state_root = cli.expected_root;
    let mut account_state = load_initial_account_state(&ee_params, input_snapshot.as_ref())?;
    let inner_root_comparison = resolve_inner_root_comparison(
        &config,
        ee_params.account_id(),
        account_state.as_mut(),
        &blobs,
        &replay_summary,
    )
    .await?;

    if let Some(path) = cli.export_snapshot_path.as_deref() {
        let exported_snapshot = build_export_snapshot(
            input_snapshot.as_ref(),
            &blobs,
            &replay_summary,
            account_state.as_ref(),
        )?;
        write_snapshot(path, &exported_snapshot)?;
    }

    Ok(Report::new(ReportInput {
        scan_stats: scan_output.stats,
        envelope_count,
        blobs_reassembled: blobs.len() as u64,
        replay_summary,
        replay_start,
        expected_state_root,
        inner_root_comparison,
    }))
}

async fn resolve_inner_root_comparison(
    config: &VerifierConfig,
    account_id: AccountId,
    account_state: Option<&mut EeAccountState>,
    blobs: &[alpen_ee_da_types::DaBlob],
    replay_summary: &ReplaySummary,
) -> Result<Option<InnerRootComparison>, DisplayedError> {
    let Some(ol_rpc_url) = config.ol_rpc_url.as_deref() else {
        return Ok(None);
    };

    let Some(applied) = replay_summary.applied() else {
        return Ok(None);
    };

    let Some(account_state) = account_state else {
        return Err(DisplayedError::UserError(
            "published inner-root comparison requires a genesis replay".to_string(),
            Box::new(PublishedComparisonError::SnapshotAccountStateUnavailable),
        ));
    };

    let per_blob_state_roots = replay_summary.per_blob_state_roots();
    if per_blob_state_roots.len() != blobs.len() {
        return Err(DisplayedError::InternalError(
            "replay summary state-root count does not match input blob count".to_string(),
            Box::new(PublishedComparisonError::PerBlobStateRootCountMismatch {
                state_root_count: per_blob_state_roots.len(),
                blob_count: blobs.len(),
            }),
        ));
    }

    let mut final_inner_roots = None;
    let mut first_inner_root_mismatch = None;
    for (blob, reconstructed_state_root) in blobs.iter().zip(per_blob_state_roots.iter().copied()) {
        let published_inner_root = apply_published_update_and_get_inner_root(
            ol_rpc_url,
            account_id,
            account_state,
            blob.update_seq_no,
            reconstructed_state_root,
        )
        .await?;
        let computed_inner_root = compute_account_inner_root(account_state);
        final_inner_roots = Some((computed_inner_root, published_inner_root));
        if computed_inner_root != published_inner_root && first_inner_root_mismatch.is_none() {
            first_inner_root_mismatch = Some((
                blob.update_seq_no,
                computed_inner_root,
                published_inner_root,
            ));
        }
    }

    let Some((final_reconstructed_inner_root, final_expected_inner_root)) = final_inner_roots
    else {
        return Ok(None);
    };

    if applied.count() != blobs.len() {
        return Err(DisplayedError::InternalError(
            "applied DA blob count does not match input blob count".to_string(),
            Box::new(PublishedComparisonError::AppliedBlobCountMismatch {
                applied_count: applied.count(),
                blob_count: blobs.len(),
            }),
        ));
    }

    let (
        reconstructed_inner_state_root,
        expected_inner_state_root,
        inner_state_root_matches_expected,
        mismatch_update_seq_no,
    ) = match first_inner_root_mismatch {
        Some((update_seq_no, reconstructed_root, expected_root)) => (
            reconstructed_root,
            expected_root,
            false,
            Some(update_seq_no),
        ),
        None => (
            final_reconstructed_inner_root,
            final_expected_inner_root,
            true,
            None,
        ),
    };

    Ok(Some(InnerRootComparison {
        reconstructed_inner_state_root,
        expected_inner_state_root,
        inner_state_root_matches_expected,
        mismatch_update_seq_no,
    }))
}

#[derive(Debug, thiserror::Error)]
enum PublishedComparisonError {
    #[error("snapshot replay does not carry EE account state")]
    SnapshotAccountStateUnavailable,

    #[error("invalid snapshot EE account state: {source}")]
    InvalidSnapshotAccountState {
        #[source]
        source: DecodeError,
    },

    #[error("applied blob count {applied_count} does not match blob count {blob_count}")]
    AppliedBlobCountMismatch {
        applied_count: usize,
        blob_count: usize,
    },

    #[error("per-blob state-root count {state_root_count} does not match blob count {blob_count}")]
    PerBlobStateRootCountMismatch {
        state_root_count: usize,
        blob_count: usize,
    },
}

#[derive(Debug, thiserror::Error)]
enum EeParamsGenesisMismatch {
    #[error("EE params genesis blockhash {params_blockhash} does not match chain genesis blockhash {chain_blockhash}")]
    Blockhash {
        params_blockhash: String,
        chain_blockhash: String,
    },

    #[error("EE params genesis state root {params_stateroot} does not match chain genesis state root {chain_stateroot}")]
    StateRoot {
        params_stateroot: String,
        chain_stateroot: String,
    },

    #[error("EE params genesis block number {params_blocknum} does not match chain genesis block number {chain_blocknum}")]
    BlockNumber {
        params_blocknum: u64,
        chain_blocknum: u64,
    },
}

fn create_genesis_reconstructor(custom_chain: &str) -> Result<StateReconstructor, DisplayedError> {
    StateReconstructor::from_chain_spec(custom_chain).user_error(format!(
        "failed to initialize EE genesis from {custom_chain}"
    ))
}

fn load_ee_params(config: &VerifierConfig) -> Result<AlpenEeParams, DisplayedError> {
    let json = fs::read_to_string(&config.ee_params).user_error(format!(
        "failed to read EE params {}",
        config.ee_params.display()
    ))?;
    AlpenEeParams::from_json_str(&json).user_error(format!(
        "failed to parse EE params {}",
        config.ee_params.display()
    ))
}

fn validate_ee_params_genesis(
    ee_params: &AlpenEeParams,
    custom_chain: &str,
) -> Result<(), DisplayedError> {
    let chain_spec = chain_value_parser(custom_chain)
        .user_error(format!("failed to load EE chain spec {custom_chain}"))?;
    let genesis_info = ee_genesis_block_info(&chain_spec);

    if ee_params.genesis_blockhash() != genesis_info.blockhash() {
        return Err(DisplayedError::UserError(
            "EE params do not match selected chain spec".to_string(),
            Box::new(EeParamsGenesisMismatch::Blockhash {
                params_blockhash: ee_params.genesis_blockhash().to_string(),
                chain_blockhash: genesis_info.blockhash().to_string(),
            }),
        ));
    }

    if ee_params.genesis_stateroot() != genesis_info.stateroot() {
        return Err(DisplayedError::UserError(
            "EE params do not match selected chain spec".to_string(),
            Box::new(EeParamsGenesisMismatch::StateRoot {
                params_stateroot: ee_params.genesis_stateroot().to_string(),
                chain_stateroot: genesis_info.stateroot().to_string(),
            }),
        ));
    }

    if ee_params.genesis_blocknum() != genesis_info.blocknum() {
        return Err(DisplayedError::UserError(
            "EE params do not match selected chain spec".to_string(),
            Box::new(EeParamsGenesisMismatch::BlockNumber {
                params_blocknum: ee_params.genesis_blocknum(),
                chain_blocknum: genesis_info.blocknum(),
            }),
        ));
    }

    Ok(())
}

fn load_initial_account_state(
    ee_params: &AlpenEeParams,
    input_snapshot: Option<&alpen_ee_da_state_replay::ReplayPreStateSnapshot>,
) -> Result<Option<EeAccountState>, DisplayedError> {
    let Some(snapshot) = input_snapshot else {
        return Ok(Some(build_genesis_ee_account_state(ee_params)));
    };

    snapshot
        .ee_account_state_ssz()
        .map(EeAccountState::from_ssz_bytes)
        .transpose()
        .map_err(|source| {
            DisplayedError::UserError(
                "invalid replay snapshot EE account state".to_string(),
                Box::new(PublishedComparisonError::InvalidSnapshotAccountState { source }),
            )
        })
}

fn map_replay_error_to_displayed(error: ReplayError) -> DisplayedError {
    match error {
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
