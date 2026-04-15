//! OL-backed expected-root lookup.

use jsonrpsee::{core::client::Error as RpcClientError, http_client::HttpClientBuilder};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_codec::{decode_buf_exact, CodecError};
use strata_ee_acct_types::UpdateExtraData;
use strata_identifiers::{AccountId, Buf32};
use strata_ol_rpc_api::OLClientRpcClient;

/// Errors raised while deriving the expected root from OL.
#[derive(Debug, thiserror::Error)]
enum ExpectedRootLookupError {
    /// The returned manifest does not match the requested update sequence.
    #[error(
        "manifest seq_no mismatch: expected {expected_update_seq_no}, got {actual_update_seq_no}"
    )]
    SeqNoMismatch {
        expected_update_seq_no: u64,
        actual_update_seq_no: u64,
    },

    /// The manifest does not contain EE-specific update extra data.
    #[error("manifest for account {account_id} update_seq_no {update_seq_no} has no extra_data")]
    MissingExtraData {
        account_id: AccountId,
        update_seq_no: u64,
    },

    /// The manifest extra data is not an EE [`UpdateExtraData`] payload.
    #[error("failed to decode manifest extra_data for update_seq_no {update_seq_no}: {source}")]
    InvalidExtraData {
        update_seq_no: u64,
        #[source]
        source: CodecError,
    },
}

/// Fetches the published Snark account update manifest for `update_seq_no` and extracts the claimed
/// EE root.
pub(crate) async fn fetch_manifest_expected_root(
    ol_rpc_url: &str,
    account_id: AccountId,
    update_seq_no: u64,
) -> Result<Buf32, DisplayedError> {
    let client = HttpClientBuilder::default()
        .build(ol_rpc_url)
        .user_error("failed to initialize OL RPC client")?;
    let manifest = client
        .get_snark_acct_update_manifest(account_id, update_seq_no)
        .await
        .map_err(map_manifest_rpc_error)?;

    if manifest.seq_no() != update_seq_no {
        return Err(DisplayedError::InternalError(
            "Published update manifest response has unexpected sequence number".to_string(),
            Box::new(ExpectedRootLookupError::SeqNoMismatch {
                expected_update_seq_no: update_seq_no,
                actual_update_seq_no: manifest.seq_no(),
            }),
        ));
    }

    let extra_data = manifest.extra_data().ok_or_else(|| {
        DisplayedError::UserError(
            "Published update manifest lacks EE update extra_data".to_string(),
            Box::new(ExpectedRootLookupError::MissingExtraData {
                account_id,
                update_seq_no,
            }),
        )
    })?;
    let extra_data = decode_buf_exact::<UpdateExtraData>(&extra_data.0).map_err(|source| {
        DisplayedError::UserError(
            "Published update manifest extra_data is not an EE update payload".to_string(),
            Box::new(ExpectedRootLookupError::InvalidExtraData {
                update_seq_no,
                source,
            }),
        )
    })?;

    Ok(*extra_data.new_tip_state_root())
}

fn map_manifest_rpc_error(error: RpcClientError) -> DisplayedError {
    match error {
        RpcClientError::Call(_) => DisplayedError::UserError(
            "failed to fetch published update manifest for applied update".to_string(),
            Box::new(error),
        ),
        _ => DisplayedError::InternalError(
            "OL RPC request failed while fetching update manifest".to_string(),
            Box::new(error),
        ),
    }
}
