use alpen_ee_common::{
    OLBlockData, OLChainStatus, OLClient, OLClientError, OLEpochSummary, SequencerOLClient,
};
use async_trait::async_trait;
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
    RpcOLTransaction, RpcSnarkAccountUpdate, RpcTransactionAttachment, RpcTransactionPayload,
};
use strata_snark_acct_types::{SnarkAccountUpdate, UpdateInputData, UpdateStateData};

/// RPC-based OL client that communicates with an OL node via JSON-RPC.
#[derive(Debug)]
pub(crate) struct RpcOLClient {
    /// Own account id
    account_id: AccountId,
    /// RPC client
    client: ManagedWsClient,
}

impl RpcOLClient {
    #[expect(unused, reason = "waiting for OL RPC impl")]
    /// Creates a new [`RpcOLClient`] with the given account ID and RPC URL.
    pub(crate) fn new(account_id: AccountId, ol_rpc_url: impl Into<String>) -> Self {
        let client = ManagedWsClient::new_with_default_pool(WsClientConfig {
            url: ol_rpc_url.into(),
        });
        Self { account_id, client }
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
