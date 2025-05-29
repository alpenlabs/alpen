use jsonrpsee::http_client::HttpClient;
use strata_rpc_api::StrataApiClient;

use super::errors::CheckpointResult;
use crate::checkpoint_runner::errors::CheckpointError;

/// Fetches the latest checkpoint index from the sequencer client.
pub(crate) async fn fetch_latest_checkpoint_index(cl_client: &HttpClient) -> CheckpointResult<u64> {
    cl_client
        .get_latest_checkpoint_index(None)
        .await
        .map_err(|e| CheckpointError::FetchError(e.to_string()))?
        .ok_or(CheckpointError::CheckpointNotFound(0))
}
