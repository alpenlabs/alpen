//! L1 client setup, block scanning, and CLI error mapping.

use alpen_ee_da_l1_extraction::{
    fetch_range, FetchError, FetchReader, L1RangeScanner, ParsedEnvelope,
};
use alpen_ee_da_types::EE_DA_MAGIC_BYTES;
use bitcoind_async_client::{traits::Reader, Auth, Client};
use futures::StreamExt;
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

/// Fetches blocks in the range and scans each for complete EE DA envelopes.
pub(crate) async fn collect_envelopes(
    reader: &impl FetchReader,
    config: &VerifierConfig,
    start_height: L1Height,
    end_height: L1Height,
) -> Result<Vec<ParsedEnvelope>, DisplayedError> {
    let mut scanner =
        L1RangeScanner::new(MagicBytes::new(EE_DA_MAGIC_BYTES), config.sequencer_pubkey);
    let mut stream =
        fetch_range(reader, start_height, end_height).user_error("invalid block range")?;

    while let Some(item) = stream.next().await {
        let block_data = item.map_err(map_fetch_error_to_displayed)?;
        scanner
            .ingest_block(block_data.block())
            .internal_error("failed to scan L1 block for EE DA envelopes")?;
    }

    scanner
        .finish()
        .internal_error("failed to scan L1 range for EE DA envelopes")
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
