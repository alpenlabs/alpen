mod client;
pub(crate) mod fetch;

pub(crate) use client::create_ready_client;
use futures::StreamExt;
use strata_cli_common::errors::DisplayedError;
use strata_identifiers::L1Height;

/// Counts L1 blocks in the requested inclusive range.
pub(crate) async fn count_blocks(
    reader: &impl fetch::FetchReader,
    start_height: L1Height,
    end_height: L1Height,
) -> Result<u64, DisplayedError> {
    if start_height > end_height {
        return Err(DisplayedError::UserError(
            "invalid block range".to_string(),
            Box::new(fetch::InvalidBlockRange {
                start_height,
                end_height,
            }),
        ));
    }

    let mut count = 0u64;
    let mut stream = fetch::fetch_range(reader, start_height, end_height);

    while let Some(item) = stream.next().await {
        item.map_err(to_displayed_error)?;
        count += 1;
    }

    Ok(count)
}

/// Classifies a fetch error as user or internal.
fn to_displayed_error(error: fetch::FetchError) -> DisplayedError {
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
