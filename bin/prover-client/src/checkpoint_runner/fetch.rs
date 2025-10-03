use jsonrpsee::http_client::HttpClient;
use strata_rpc_api::StrataApiClient;

use super::errors::CheckpointResult;
use crate::checkpoint_runner::errors::CheckpointError;

/// Fetches the next (lowest) unproven checkpoint index from the sequencer client.
/// This keeps proofs contiguous and prevents gaps in the proven sequence.
pub(crate) async fn fetch_latest_unproven_checkpoint_index(
    cl_client: &HttpClient,
) -> CheckpointResult<Option<u64>> {
    cl_client
        .get_latest_unproven_checkpoint_index()
        .await
        .map_err(|e| CheckpointError::FetchError(e.to_string()))
}
