use alpen_ee_common::{
    OLAccountStateView, OLBlockData, OLChainStatus, OLClient, OLClientError, OLEpochSummary,
    SequencerOLClient,
};
use async_trait::async_trait;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use ssz::Encode;
use strata_common::{
    retry::{
        policies::ExponentialBackoff, retry_with_backoff_async, DEFAULT_ENGINE_CALL_MAX_RETRIES,
    },
    ws_client::{ManagedWsClient, WsClientConfig},
};
use strata_identifiers::{AccountId, Epoch};
use strata_rpc_api_new::OLClientRpcClient;
use strata_rpc_types_new::{
    OLBlockOrTag, RpcOLTransaction, RpcSnarkAccountUpdate, RpcTransactionAttachment,
    RpcTransactionPayload,
};
use strata_snark_acct_types::{ProofState, SnarkAccountUpdate, UpdateInputData, UpdateStateData};

/// RPC-based OL client that communicates with an OL node via JSON-RPC.
#[derive(Debug)]
pub(crate) struct RpcOLClient {
    /// Own account id
    account_id: AccountId,
    /// RPC client
    client: RpcTransportClient,
}

impl RpcOLClient {
    /// Creates a new [`RpcOLClient`] with the given account ID and RPC URL.
    pub(crate) fn try_new(
        account_id: AccountId,
        ol_rpc_url: impl Into<String>,
    ) -> Result<Self, OLClientError> {
        let client = RpcTransportClient::from_url(ol_rpc_url.into())?;
        Ok(Self { account_id, client })
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

impl RpcTransportClient {
    fn from_url(url: String) -> Result<Self, OLClientError> {
        if url.starts_with("http://") || url.starts_with("https://") {
            let client = HttpClientBuilder::default()
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
            WsClientConfig { url: ws_url },
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
                let status = self
                    .client
                    .chain_status()
                    .await
                    .map_err(|e| OLClientError::rpc(e.to_string()))?;

                Ok(OLChainStatus {
                    latest: *status.latest(),
                    confirmed: *status.confirmed(),
                    finalized: *status.finalized(),
                })
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
                let epoch_summary = self
                    .client
                    .get_acct_epoch_summary(self.account_id, epoch)
                    .await
                    .map_err(|e| OLClientError::rpc(e.to_string()))?;

                let update = UpdateInputData::new(
                    epoch_summary.next_seq_no,
                    epoch_summary
                        .processed_msgs
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    UpdateStateData::new(
                        epoch_summary.proof_state.into(),
                        epoch_summary.extra_data.into(),
                    ),
                );

                Ok(OLEpochSummary::new(
                    epoch_summary.epoch_commitment,
                    epoch_summary.prev_epoch_commitment,
                    vec![update],
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
                let block_summaries = self
                    .client
                    .get_blocks_summaries(self.account_id, min_slot, max_slot)
                    .await
                    .map_err(|e| OLClientError::rpc(e.to_string()))?;

                let blocks = block_summaries
                    .into_iter()
                    .map(|block_summary| OLBlockData {
                        commitment: block_summary.block_commitment,
                        inbox_messages: block_summary
                            .new_inbox_messages
                            .into_iter()
                            .map(Into::into)
                            .collect(),
                        next_inbox_msg_idx: block_summary.next_inbox_msg_idx,
                    })
                    .collect();

                Ok(blocks)
            },
        )
        .await
    }

    /// Retrieves latest account state in the OL Chain for this account.
    async fn get_latest_account_state(&self) -> Result<OLAccountStateView, OLClientError> {
        let snark_account_state = self
            .client
            .get_snark_account_state(self.account_id, OLBlockOrTag::Latest)
            .await
            .map_err(|e| OLClientError::rpc(e.to_string()))?
            .ok_or_else(|| OLClientError::Rpc("missing latest account state".into()))?;

        Ok(OLAccountStateView {
            seq_no: snark_account_state.seq_no().into(),
            proof_state: ProofState::new(
                snark_account_state.inner_state().0.into(),
                snark_account_state.next_inbox_msg_idx(),
            ),
        })
    }

    async fn submit_update(&self, update: SnarkAccountUpdate) -> Result<(), OLClientError> {
        let rpc_update = RpcSnarkAccountUpdate::new(
            (*self.account_id.inner()).into(),
            update.operation.as_ssz_bytes().into(),
            update.update_proof.to_vec().into(),
        );

        let tx = RpcOLTransaction::new(
            RpcTransactionPayload::SnarkAccountUpdate(rpc_update),
            RpcTransactionAttachment::new(None, None),
        );

        retry_with_backoff_async(
            "ol_client_submit_update",
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || async {
                self.client
                    .submit_transaction(tx.clone())
                    .await
                    .map_err(|e| OLClientError::rpc(e.to_string()))?;

                Ok(())
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{OLClientError, RpcTransportClient};

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
}
