mod client;
pub(crate) mod fetch;
pub(crate) mod scan;
pub(crate) mod walk;

#[cfg(test)]
pub(crate) mod test_utils;

use bitcoin::hashes::Hash;
pub(crate) use client::create_ready_client;
use futures::StreamExt;
use scan::{scan_block, RevealRecord};
use serde::Serialize;
use strata_cli_common::errors::DisplayedError;
use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, L1Height};
use strata_l1_txfmt::MagicBytes;

/// Per-L1-block reveal stats for blocks that contained at least one reveal.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct L1BlockRevealStats {
    pub(crate) commitment: L1BlockCommitment,
    pub(crate) reveals_found: u64,
}

/// Output of the L1 scan stage.
pub(crate) struct L1ScanOutput {
    pub(crate) fetched_block_count: u64,
    pub(crate) blocks_with_reveals: Vec<L1BlockRevealStats>,
    #[expect(
        dead_code,
        reason = "Consumed by the segmenter stage in the next commit."
    )]
    pub(crate) ordered_reveals: Vec<RevealRecord>,
}

/// Fetches blocks in the range, scans each for reveals, and walks the
/// global reveal chain so the returned reveals are in predecessor order.
pub(crate) async fn collect_reveals(
    reader: &impl fetch::FetchReader,
    start_height: L1Height,
    end_height: L1Height,
    magic_bytes: MagicBytes,
) -> Result<L1ScanOutput, DisplayedError> {
    if start_height > end_height {
        return Err(DisplayedError::UserError(
            "invalid block range".to_string(),
            Box::new(fetch::InvalidBlockRange {
                start_height,
                end_height,
            }),
        ));
    }

    let mut fetched_block_count = 0u64;
    let mut blocks_with_reveals = Vec::new();
    let mut reveals = Vec::new();
    let mut stream = fetch::fetch_range(reader, start_height, end_height);

    while let Some(item) = stream.next().await {
        let block_data = item.map_err(fetch_error_to_displayed)?;
        fetched_block_count += 1;
        let block_reveals = scan_block(&block_data.block, magic_bytes).map_err(|source| {
            DisplayedError::InternalError(
                "failed to scan block for reveals".to_string(),
                Box::new(source),
            )
        })?;
        if !block_reveals.is_empty() {
            let commitment = L1BlockCommitment::new(
                block_data.height,
                L1BlockId::from(Buf32::from(block_data.hash.to_byte_array())),
            );
            blocks_with_reveals.push(L1BlockRevealStats {
                commitment,
                reveals_found: block_reveals.len() as u64,
            });
            reveals.extend(block_reveals);
        }
    }

    let ordered_reveals = walk::walk_reveals(reveals).map_err(|source| {
        DisplayedError::InternalError("failed to walk reveal chain".to_string(), Box::new(source))
    })?;

    Ok(L1ScanOutput {
        fetched_block_count,
        blocks_with_reveals,
        ordered_reveals,
    })
}

/// Classifies a fetch error as user or internal.
fn fetch_error_to_displayed(error: fetch::FetchError) -> DisplayedError {
    match error {
        fetch::FetchError::HeightOutOfRange { .. } => {
            DisplayedError::UserError("requested height out of range".to_string(), Box::new(error))
        }
        fetch::FetchError::RetriesExhausted { .. } => DisplayedError::InternalError(
            "retries exhausted while fetching L1 blocks".to_string(),
            Box::new(error),
        ),
        fetch::FetchError::Client { .. } => DisplayedError::InternalError(
            "bitcoind client error while fetching L1 blocks".to_string(),
            Box::new(error),
        ),
    }
}
