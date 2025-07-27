use alloy_eips::eip7685::RequestsOrHash;
use alloy_rpc_types::{
    engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, JwtSecret, PayloadId},
    eth::{Block as RpcBlock, Header, Transaction, TransactionRequest},
};
use alpen_reth_node::{AlpenEngineTypes, AlpenExecutionPayloadEnvelopeV4, AlpenPayloadAttributes};
use jsonrpsee::{core::client::SubscriptionClientT, http_client::HttpClientBuilder};
#[cfg(test)]
use mockall::automock;
use reth_primitives::Receipt;
use reth_rpc_api::{EngineApiClient, EthApiClient};
use reth_rpc_layer::AuthClientLayer;
use revm_primitives::alloy_primitives::{BlockHash, B256};

type RpcResult<T> = Result<T, jsonrpsee::core::ClientError>;

#[allow(async_fn_in_trait)]
#[cfg_attr(test, automock)]
pub trait EngineRpc {
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<AlpenPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated>;

    async fn get_payload_v4(
        &self,
        payload_id: PayloadId,
    ) -> RpcResult<AlpenExecutionPayloadEnvelopeV4>;

    async fn new_payload_v4(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        execution_requests: RequestsOrHash,
    ) -> RpcResult<alloy_rpc_types::engine::PayloadStatus>;

    async fn block_by_hash(&self, block_hash: BlockHash) -> RpcResult<Option<RpcBlock>>;
}

#[derive(Debug, Clone)]
pub struct EngineRpcClient {
    http_url: String,
    secret: JwtSecret,
}

impl EngineRpcClient {
    pub fn from_url_secret(http_url: &str, secret: JwtSecret) -> Self {
        EngineRpcClient {
            http_url: http_url.to_string(),
            secret,
        }
    }

    fn create_client(&self) -> impl SubscriptionClientT + Clone + Send + Sync + Unpin + 'static {
        let middleware = tower::ServiceBuilder::new().layer(AuthClientLayer::new(self.secret));
        HttpClientBuilder::default()
            .set_http_middleware(middleware)
            .build(&self.http_url)
            .expect("Failed to create http client")
    }
}

impl EngineRpc for EngineRpcClient {
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<AlpenPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        let client = self.create_client();
        EngineApiClient::<AlpenEngineTypes>::fork_choice_updated_v3(
            &client,
            fork_choice_state,
            payload_attributes,
        )
        .await
    }

    async fn get_payload_v4(
        &self,
        payload_id: PayloadId,
    ) -> RpcResult<AlpenExecutionPayloadEnvelopeV4> {
        let client = self.create_client();
        EngineApiClient::<AlpenEngineTypes>::get_payload_v4(&client, payload_id).await
    }

    async fn new_payload_v4(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        execution_requests: RequestsOrHash,
    ) -> RpcResult<alloy_rpc_types::engine::PayloadStatus> {
        let client = self.create_client();
        EngineApiClient::<AlpenEngineTypes>::new_payload_v4(
            &client,
            payload,
            versioned_hashes,
            parent_beacon_block_root,
            execution_requests,
        )
        .await
    }

    async fn block_by_hash(&self, block_hash: BlockHash) -> RpcResult<Option<RpcBlock>> {
        let client = self.create_client();
        EthApiClient::<
            TransactionRequest,
            Transaction,
            RpcBlock<alloy_rpc_types::Transaction>,
            Receipt,
            Header,
        >::block_by_hash(&client, block_hash, false)
        .await
    }
}
