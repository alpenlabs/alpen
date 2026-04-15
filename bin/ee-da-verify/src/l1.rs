//! L1 client setup, block scanning, and CLI error mapping.

use bitcoin::hashes::Hash;
use bitcoind_async_client::{traits::Reader, Auth, Client};
use ee_da_l1::{fetch_range, FetchError, FetchReader, ParsedEnvelope};
use futures::StreamExt;
use serde::Serialize;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, L1Height};

use crate::config::VerifierConfig;

const DISABLE_CLIENT_RETRIES: u8 = 0;

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

/// Per-L1-block envelope stats for blocks that contained at least one envelope.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct L1BlockEnvelopeStats {
    pub(crate) commitment: L1BlockCommitment,
    pub(crate) envelopes_found: u64,
}

/// Aggregate stats from the bounded L1 scan stage.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct L1ScanStats {
    pub(crate) fetched_block_count: u64,
    pub(crate) blocks_with_envelopes: Vec<L1BlockEnvelopeStats>,
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
    let mut fetched_block_count = 0u64;
    let mut blocks_with_envelopes = Vec::new();
    let mut envelopes = Vec::new();
    let mut stream =
        fetch_range(reader, start_height, end_height).user_error("invalid block range")?;

    while let Some(item) = stream.next().await {
        let block_data = item.map_err(fetch_error_to_displayed)?;
        fetched_block_count += 1;

        let block_envelopes = ee_da_l1::scan_block(
            block_data.block(),
            config.magic_bytes,
            config.sequencer_pubkey,
        )
        .internal_error("failed to scan block for EE DA envelopes")?;

        if !block_envelopes.is_empty() {
            blocks_with_envelopes.push(L1BlockEnvelopeStats {
                commitment: L1BlockCommitment::new(
                    block_data.height(),
                    L1BlockId::from(Buf32::from(block_data.hash().to_byte_array())),
                ),
                envelopes_found: block_envelopes.len() as u64,
            });
            envelopes.extend(block_envelopes);
        }
    }

    Ok(L1ScanOutput {
        stats: L1ScanStats {
            fetched_block_count,
            blocks_with_envelopes,
        },
        envelopes,
    })
}

fn fetch_error_to_displayed(error: FetchError) -> DisplayedError {
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
