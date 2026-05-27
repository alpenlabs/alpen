use alpen_ee_common::{
    EpochUpdateOp, OLAccountStateView, OLBlockData, OLChainStatus, OLClient, OLClientError,
    OLEpochSummary, SequencerOLClient,
};
use async_trait::async_trait;
use http::{header::AUTHORIZATION, HeaderMap, HeaderValue};
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use ssz::Encode;
use strata_acct_types::MessageEntry;
use strata_common::{
    retry::{
        policies::ExponentialBackoff, retry_with_backoff_async, DEFAULT_ENGINE_CALL_MAX_RETRIES,
    },
    ws_client::{ManagedWsClient, WsClientConfig},
};
use strata_identifiers::{
    AccountId, Epoch, EpochCommitment, Hash, L1Height, OLBlockCommitment, OLTxId,
};
use strata_ol_rpc_api::{OLClientRpcClient, OLSubmitRpcClient};
use strata_ol_rpc_types::{
    OLBlockOrTag, RpcOLTransaction, RpcSnarkAccountUpdate, RpcTransactionPayload, RpcTxConstraints,
};
use strata_snark_acct_types::{ProofState, SnarkAccountUpdate};
use tracing::info;

/// Max retries for startup RPC calls where the OL node may still be booting.
const STARTUP_RPC_MAX_RETRIES: u16 = 10;

/// RPC-based OL client that communicates with an OL node via JSON-RPC.
#[derive(Debug)]
pub(crate) struct RpcOLClient {
    /// Own account id
    account_id: AccountId,
    /// RPC client used for read-only OL calls.
    read_client: RpcTransportClient,
    /// RPC client used for authenticated OL transaction submission.
    submit_client: RpcTransportClient,
}

impl RpcOLClient {
    /// Creates a new [`RpcOLClient`] with the given account ID and RPC URL.
    pub(crate) fn try_new(
        account_id: AccountId,
        ol_rpc_url: impl Into<String>,
        ol_submit_url: Option<&str>,
        ol_submit_bearer_token: Option<&str>,
    ) -> Result<Self, OLClientError> {
        let ol_rpc_url = ol_rpc_url.into();
        let read_client = RpcTransportClient::from_url(ol_rpc_url.clone())?;
        let submit_client = match ol_submit_url {
            Some(url) => {
                let token = ol_submit_bearer_token.ok_or_else(|| {
                    OLClientError::rpc("--ol-submit-bearer-token is required with --ol-submit-url")
                })?;
                RpcTransportClient::from_url_with_headers(
                    url.to_string(),
                    bearer_auth_headers(token)?,
                )?
            }
            None => RpcTransportClient::from_url(ol_rpc_url)?,
        };
        Ok(Self {
            account_id,
            read_client,
            submit_client,
        })
    }
}

/// Transport-agnostic RPC client for the OL node.
#[derive(Debug)]
enum RpcTransportClient {
    /// WebSocket client
    Ws(ManagedWsClient),
    /// HTTP client
    Http(HttpClient),
}

/// Dispatches an RPC method call to the underlying transport client (WS or HTTP),
/// mapping any RPC error to [`OLClientError`].
macro_rules! call_rpc_on {
    ($client:expr, $method:ident($($args:expr),*)) => {
        match $client {
            RpcTransportClient::Ws(client) => client
                .$method($($args),*)
                .await
                .map_err(|e| OLClientError::rpc(e.to_string())),
            RpcTransportClient::Http(client) => client
                .$method($($args),*)
                .await
                .map_err(|e| OLClientError::rpc(e.to_string())),
        }
    };
}

macro_rules! call_read_rpc {
    ($self:expr, $method:ident($($args:expr),*)) => {
        call_rpc_on!(&$self.read_client, $method($($args),*))
    };
}

macro_rules! call_submit_rpc {
    ($self:expr, $method:ident($($args:expr),*)) => {
        call_rpc_on!(&$self.submit_client, $method($($args),*))
    };
}

fn bearer_auth_headers(token: &str) -> Result<HeaderMap, OLClientError> {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|e| OLClientError::rpc(e.to_string()))?;
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

impl RpcTransportClient {
    fn from_url(url: String) -> Result<Self, OLClientError> {
        Self::from_url_with_headers(url, HeaderMap::new())
    }

    fn from_url_with_headers(url: String, headers: HeaderMap) -> Result<Self, OLClientError> {
        if url.starts_with("http://") || url.starts_with("https://") {
            let client = HttpClientBuilder::default()
                .set_headers(headers)
                .build(&url)
                .map_err(|e| OLClientError::rpc(e.to_string()))?;
            return Ok(Self::Http(client));
        }

        let ws_url = if url.starts_with("ws://") || url.starts_with("wss://") {
            url
        } else if url.contains("://") {
            return Err(OLClientError::rpc(format!(
                "unsupported OL RPC scheme: {url}"
            )));
        } else {
            // Default to WebSocket when no scheme is provided.
            format!("ws://{url}")
        };

        Ok(Self::Ws(ManagedWsClient::new_with_default_pool(
            WsClientConfig {
                url: ws_url,
                headers,
            },
        )))
    }
}

#[async_trait]
impl OLClient for RpcOLClient {
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError> {
        retry_with_backoff_async(
            "ol_client_chain_status",
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || async {
                let status = call_read_rpc!(self, chain_status())?;

                Ok(OLChainStatus {
                    tip: OLBlockCommitment::new(status.tip().slot(), status.tip().blkid()),
                    confirmed: *status.confirmed(),
                    finalized: *status.finalized(),
                    latest: *status.latest(),
                })
            },
        )
        .await
    }

    async fn account_genesis_epoch(&self) -> Result<EpochCommitment, OLClientError> {
        retry_with_backoff_async(
            "ol_client_account_genesis_epoch",
            STARTUP_RPC_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || async {
                call_read_rpc!(self, get_account_genesis_epoch_commitment(self.account_id))
            },
        )
        .await
    }

    async fn epoch_summary(&self, epoch: Epoch) -> Result<OLEpochSummary, OLClientError> {
        retry_with_backoff_async(
            "ol_client_epoch_summary",
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || async {
                let epoch_summary =
                    call_read_rpc!(self, get_acct_epoch_summary(self.account_id, epoch))?;

                let updates: Vec<EpochUpdateOp> = epoch_summary
                    .update_inputs()
                    .iter()
                    .map(|u| {
                        let messages = u
                            .messages
                            .iter()
                            .cloned()
                            .map(MessageEntry::try_from)
                            .collect::<Result<_, _>>()
                            .map_err(|e| OLClientError::rpc(e.to_string()))?;
                        Ok(EpochUpdateOp::new(
                            u.seq_no,
                            u.extra_data.0.clone(),
                            messages,
                            u.final_state_root.as_ref().map(|root| root.0.into()),
                        ))
                    })
                    .collect::<Result<_, OLClientError>>()?;

                Ok(OLEpochSummary::new(
                    epoch_summary.epoch_commitment(),
                    epoch_summary.prev_epoch_commitment(),
                    updates,
                ))
            },
        )
        .await
    }
}

#[async_trait]
impl SequencerOLClient for RpcOLClient {
    async fn chain_status(&self) -> Result<OLChainStatus, OLClientError> {
        <Self as OLClient>::chain_status(self).await
    }

    async fn get_inbox_messages(
        &self,
        min_slot: u64,
        max_slot: u64,
    ) -> Result<Vec<OLBlockData>, OLClientError> {
        retry_with_backoff_async(
            "ol_client_get_inbox_messages",
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || async {
                let block_summaries = call_read_rpc!(
                    self,
                    get_blocks_summaries(self.account_id, min_slot, max_slot)
                )?;

                let blocks = block_summaries
                    .into_iter()
                    .map(|block_summary| {
                        let inbox_messages = block_summary
                            .new_inbox_messages
                            .into_iter()
                            .map(MessageEntry::try_from)
                            .collect::<Result<_, _>>()
                            .map_err(|e| OLClientError::rpc(e.to_string()))?;

                        Ok(OLBlockData {
                            commitment: block_summary.block_commitment,
                            inbox_messages,
                            next_inbox_msg_idx: block_summary.next_inbox_msg_idx,
                        })
                    })
                    .collect::<Result<_, OLClientError>>()?;

                Ok(blocks)
            },
        )
        .await
    }

    /// Retrieves latest account state in the OL Chain for this account.
    async fn get_latest_account_state(&self) -> Result<OLAccountStateView, OLClientError> {
        let snark_account_state = call_read_rpc!(
            self,
            get_snark_account_state(self.account_id, OLBlockOrTag::Latest)
        )?
        .ok_or_else(|| OLClientError::Rpc("missing latest account state".into()))?;

        Ok(OLAccountStateView {
            seq_no: snark_account_state.seq_no().into(),
            proof_state: ProofState::new(
                snark_account_state.inner_state().0.into(),
                snark_account_state.next_inbox_msg_idx(),
            ),
        })
    }

    async fn get_asm_manifest_commitment(
        &self,
        l1_height: L1Height,
    ) -> Result<Hash, OLClientError> {
        retry_with_backoff_async(
            "ol_client_get_asm_manifest_commitment",
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || async {
                let commitment = call_read_rpc!(self, get_asm_manifest_commitment(l1_height))?;

                commitment.map(|h| Hash::from(h.0)).ok_or_else(|| {
                    OLClientError::rpc(format!(
                        "missing L1 header commitment for L1 height {l1_height}"
                    ))
                })
            },
        )
        .await
    }

    async fn submit_update(&self, update: SnarkAccountUpdate) -> Result<OLTxId, OLClientError> {
        let operation = update.operation();
        let seq_no = operation.seq_no();
        let inner_state = operation.new_proof_state().inner_state();
        let next_inbox_msg_idx = operation.new_proof_state().next_inbox_msg_idx();
        let l1_ref_heights: Vec<_> = operation
            .ledger_refs()
            .l1_block_refs()
            .iter()
            .map(|claim| claim.idx())
            .collect();
        let outputs = operation.outputs();
        let output_transfer_count = outputs.transfers().len();
        let output_message_count = outputs.messages().len();
        let output_message_value_sats: u64 = outputs
            .messages()
            .iter()
            .map(|message| message.payload().value().to_sat())
            .sum();
        let extra_data_len = operation.extra_data().len();

        let rpc_update = RpcSnarkAccountUpdate::new(
            (*self.account_id.inner()).into(),
            operation.as_ssz_bytes().into(),
            update.update_proof().to_vec().into(),
        );

        let tx = RpcOLTransaction::new(
            RpcTransactionPayload::SnarkAccountUpdate(rpc_update),
            RpcTxConstraints::new(None, None),
        );

        let txid = retry_with_backoff_async(
            "ol_client_submit_update",
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || async { call_submit_rpc!(self, submit_transaction(tx.clone())) },
        )
        .await?;

        info!(
            account_id = %self.account_id,
            %txid,
            seq_no,
            %inner_state,
            next_inbox_msg_idx,
            extra_data_len,
            output_transfer_count,
            output_message_count,
            output_message_value_sats,
            l1_ref_count = l1_ref_heights.len(),
            ?l1_ref_heights,
            "submitted snark update to OL"
        );

        Ok(txid)
    }
}

#[cfg(test)]
mod tests {
    use http::header::HeaderName;
    use strata_identifiers::AccountId;

    use super::{bearer_auth_headers, OLClientError, RpcOLClient, RpcTransportClient};

    #[test]
    fn http_url_uses_http_client() {
        let client = RpcTransportClient::from_url("http://localhost:1234".to_string()).unwrap();
        assert!(matches!(client, RpcTransportClient::Http(_)));
    }

    #[test]
    fn https_url_uses_http_client() {
        let client = RpcTransportClient::from_url("https://localhost:1234".to_string()).unwrap();
        assert!(matches!(client, RpcTransportClient::Http(_)));
    }

    #[test]
    fn ws_url_uses_ws_client() {
        let client = RpcTransportClient::from_url("ws://localhost:1234".to_string()).unwrap();
        assert!(matches!(client, RpcTransportClient::Ws(_)));
    }

    #[test]
    fn wss_url_uses_ws_client() {
        let client = RpcTransportClient::from_url("wss://localhost:1234".to_string()).unwrap();
        assert!(matches!(client, RpcTransportClient::Ws(_)));
    }

    #[test]
    fn no_scheme_defaults_to_ws() {
        let client = RpcTransportClient::from_url("localhost:1234".to_string()).unwrap();
        assert!(matches!(client, RpcTransportClient::Ws(_)));
    }

    #[test]
    fn unsupported_scheme_errors() {
        let err = RpcTransportClient::from_url("ftp://localhost:1234".to_string())
            .expect_err("expected unsupported scheme to fail");
        match err {
            OLClientError::Rpc(msg) => {
                assert!(msg.contains("unsupported OL RPC scheme"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bearer_auth_headers_sets_authorization() {
        let headers = bearer_auth_headers("test-token").unwrap();
        assert_eq!(
            headers
                .get(HeaderName::from_static("authorization"))
                .unwrap(),
            "Bearer test-token"
        );
    }

    #[test]
    fn submit_url_requires_bearer_token() {
        let err = RpcOLClient::try_new(
            AccountId::new([0u8; 32]),
            "http://localhost:1234",
            Some("http://localhost:1235"),
            None,
        )
        .expect_err("missing submit token should fail");

        match err {
            OLClientError::Rpc(msg) => {
                assert!(msg.contains("--ol-submit-bearer-token"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
