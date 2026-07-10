//! L1 client setup, block scanning, and CLI error mapping.

use alpen_ee_da_l1_extraction::{
    fetch_range, FetchError, FetchReader, L1RangeScanner, ParsedEnvelope,
};
use alpen_ee_da_types::EE_DA_MAGIC_BYTES;
use bitcoind_async_client::{traits::Reader, Auth, Client};
use futures::StreamExt;
use serde::Serialize;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_identifiers::L1Height;
use strata_l1_txfmt::MagicBytes;

use crate::config::VerifierConfig;

const DISABLE_CLIENT_RETRIES: u16 = 0;

/// Builds a bitcoind client and verifies readiness before fetch starts.
pub(crate) async fn create_ready_client(config: &VerifierConfig) -> Result<Client, DisplayedError> {
    let client = Client::new(
        config.bitcoind_url.clone(),
        Auth::UserPass(
            config.bitcoind_rpc_user.clone(),
            config.bitcoind_rpc_password.clone(),
        ),
        Some(DISABLE_CLIENT_RETRIES),
        None,
        None,
    )
    .user_error("failed to initialize bitcoind client")?;

    Reader::get_blockchain_info(&client)
        .await
        .internal_error("bitcoind not ready for fetch")?;

    Ok(client)
}

/// Aggregate stats from the bounded L1 scan stage.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct L1ScanStats {
    pub(crate) fetched_block_count: u64,
}

/// Output of the bounded L1 scan stage.
pub(crate) struct L1ScanOutput {
    pub(crate) stats: L1ScanStats,
    pub(crate) envelopes: Vec<ParsedEnvelope>,
}

/// Fetches blocks in the range and scans each for complete EE DA envelopes.
pub(crate) async fn collect_envelopes(
    reader: &impl FetchReader,
    config: &VerifierConfig,
    start_height: L1Height,
    end_height: L1Height,
) -> Result<L1ScanOutput, DisplayedError> {
    let mut scanner =
        L1RangeScanner::new(MagicBytes::new(EE_DA_MAGIC_BYTES), config.sequencer_pubkey);
    let mut fetched_block_count = 0u64;
    let mut stream =
        fetch_range(reader, start_height, end_height).user_error("invalid block range")?;

    while let Some(item) = stream.next().await {
        let block_data = item.map_err(map_fetch_error_to_displayed)?;
        fetched_block_count += 1;
        scanner
            .ingest_block(block_data.block())
            .internal_error("failed to scan L1 block for EE DA envelopes")?;
    }

    let envelopes = scanner
        .finish()
        .internal_error("failed to scan L1 range for EE DA envelopes")?;

    Ok(L1ScanOutput {
        stats: L1ScanStats {
            fetched_block_count,
        },
        envelopes,
    })
}

fn map_fetch_error_to_displayed(error: FetchError) -> DisplayedError {
    match error {
        FetchError::HeightOutOfRange { .. } => {
            DisplayedError::UserError("requested height out of range".to_string(), Box::new(error))
        }
        FetchError::RetriesExhausted { .. } => DisplayedError::InternalError(
            "retries exhausted while fetching L1 blocks".to_string(),
            Box::new(error),
        ),
        FetchError::Client { .. } => DisplayedError::InternalError(
            "bitcoind client error while fetching L1 blocks".to_string(),
            Box::new(error),
        ),
    }
}
