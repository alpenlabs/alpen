//! OL-backed published-root lookup.

use jsonrpsee::{core::client::Error as RpcClientError, http_client::HttpClientBuilder};
use strata_acct_types::{AccountId, MessageEntry};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_codec::{decode_buf_exact, CodecError};
use strata_ee_acct_runtime::apply_input_messages;
use strata_ee_acct_types::{EeAccountState, EnvError, UpdateExtraData};
use strata_identifiers::Buf32;
use strata_ol_rpc_api::OLClientRpcClient;
use strata_ol_rpc_types::{RpcIndexedEntry, RpcMessageEntry};
use tree_hash::{Sha256Hasher, TreeHash};

/// Errors raised while deriving the published root from OL.
#[derive(Debug, thiserror::Error)]
enum PublishedRootLookupError {
    /// The returned manifest does not match the requested update sequence.
    #[error(
        "manifest seq_no mismatch: expected {expected_update_seq_no}, got {actual_update_seq_no}"
    )]
    SeqNoMismatch {
        expected_update_seq_no: u64,
        actual_update_seq_no: u64,
    },

    /// The manifest does not contain a stored Snark account inner root.
    #[error("manifest for account {account_id} update_seq_no {update_seq_no} has no inner root")]
    MissingInnerRoot {
        account_id: AccountId,
        update_seq_no: u64,
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

    /// An inbox message returned by OL cannot be converted to an account message.
    #[error("failed to decode inbox message {message_index} for update_seq_no {update_seq_no}: {source}")]
    InvalidInboxMessage {
        update_seq_no: u64,
        message_index: u64,
        #[source]
        source: strata_acct_types::MsgPayloadError,
    },

    /// Applying update inputs to the local EE account state failed.
    #[error("failed to apply inbox messages for update_seq_no {update_seq_no}: {source}")]
    ApplyInboxMessages {
        update_seq_no: u64,
        #[source]
        source: EnvError,
    },
}

/// Applies the published update metadata to `account_state` and returns the OL-published inner
/// root.
pub(crate) async fn apply_published_update_and_get_inner_root(
    ol_rpc_url: &str,
    account_id: AccountId,
    account_state: &mut EeAccountState,
    update_seq_no: u64,
    reconstructed_state_root: Buf32,
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
            Box::new(PublishedRootLookupError::SeqNoMismatch {
                expected_update_seq_no: update_seq_no,
                actual_update_seq_no: manifest.seq_no(),
            }),
        ));
    }

    let published_inner_root = manifest.new_inner_state_root().ok_or_else(|| {
        DisplayedError::UserError(
            "Published update manifest lacks inner state root".to_string(),
            Box::new(PublishedRootLookupError::MissingInnerRoot {
                account_id,
                update_seq_no,
            }),
        )
    })?;
    let extra_data = manifest.extra_data().ok_or_else(|| {
        DisplayedError::UserError(
            "Published update manifest lacks EE update extra_data".to_string(),
            Box::new(PublishedRootLookupError::MissingExtraData {
                account_id,
                update_seq_no,
            }),
        )
    })?;
    let extra_data = decode_buf_exact::<UpdateExtraData>(&extra_data.0).map_err(|source| {
        DisplayedError::UserError(
            "Published update manifest extra_data is not an EE update payload".to_string(),
            Box::new(PublishedRootLookupError::InvalidExtraData {
                update_seq_no,
                source,
            }),
        )
    })?;

    let inbox_messages = client
        .get_snark_acct_inbox_msg_range(
            account_id,
            manifest.prev_next_msg_idx(),
            manifest.new_next_msg_idx(),
        )
        .await
        .map_err(map_inbox_rpc_error)?;
    let messages = rpc_messages_to_entries(update_seq_no, inbox_messages)?;

    apply_input_messages(account_state, &messages).map_err(|source| {
        DisplayedError::InternalError(
            "failed to apply published inbox messages".to_string(),
            Box::new(PublishedRootLookupError::ApplyInboxMessages {
                update_seq_no,
                source,
            }),
        )
    })?;
    account_state.set_last_exec_blkid(Buf32(*extra_data.new_tip_blkid().as_ref()));
    account_state.set_last_exec_state_root(reconstructed_state_root);
    account_state.remove_pending_inputs(*extra_data.processed_inputs() as usize);
    account_state.remove_pending_fincls(*extra_data.processed_fincls() as usize);

    Ok(Buf32(published_inner_root.0))
}

/// Computes the SSZ inner root for the reconstructed EE account state.
pub(crate) fn compute_account_inner_root(account_state: &EeAccountState) -> Buf32 {
    <EeAccountState as TreeHash>::tree_hash_root::<Sha256Hasher>(account_state).into()
}

fn rpc_messages_to_entries(
    update_seq_no: u64,
    messages: Vec<RpcIndexedEntry<RpcMessageEntry>>,
) -> Result<Vec<MessageEntry>, DisplayedError> {
    messages
        .into_iter()
        .map(|message| {
            let message_index = message.index();
            message.value().clone().try_into().map_err(|source| {
                DisplayedError::UserError(
                    "Published update inbox message is invalid".to_string(),
                    Box::new(PublishedRootLookupError::InvalidInboxMessage {
                        update_seq_no,
                        message_index,
                        source,
                    }),
                )
            })
        })
        .collect()
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

fn map_inbox_rpc_error(error: RpcClientError) -> DisplayedError {
    match error {
        RpcClientError::Call(_) => DisplayedError::UserError(
            "failed to fetch published inbox messages for applied update".to_string(),
            Box::new(error),
        ),
        _ => DisplayedError::InternalError(
            "OL RPC request failed while fetching inbox messages".to_string(),
            Box::new(error),
        ),
    }
}
