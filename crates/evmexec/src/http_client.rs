use alloy_eips::eip7685::RequestsOrHash;
use alloy_rpc_types::{
    engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, JwtSecret, PayloadId},
    eth::{Block as RpcBlock, Header, Transaction, TransactionRequest},
};
use alpen_reth_node::{AlpenEngineTypes, AlpenExecutionPayloadEnvelopeV4, AlpenPayloadAttributes};
use http::header::AUTHORIZATION;
use jsonrpsee::ws_client::{HeaderMap, WsClient};
#[cfg(test)]
use mockall::automock;
use reth_primitives::Receipt;
use reth_rpc_api::{EngineApiClient, EthApiClient};
use reth_rpc_layer::secret_to_bearer_header;
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

#[derive(Debug)]
pub struct EngineRpcClient {
    ws_client: WsClient,
}

impl EngineRpcClient {
    pub async fn from_url_secret(wss_url: &str, secret: JwtSecret) -> Self {
        let ws_client = Self::ws_client(secret, wss_url).await;
        Self { ws_client }
    }

    pub async fn ws_client(secret: JwtSecret, http_url: &str) -> jsonrpsee::ws_client::WsClient {
        jsonrpsee::ws_client::WsClientBuilder::default()
            .set_headers(HeaderMap::from_iter([(
                AUTHORIZATION,
                secret_to_bearer_header(&secret),
            )]))
            .build(http_url)
            .await
            .expect("Failed to create ws client")
    }
}

impl EngineRpc for EngineRpcClient {
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<AlpenPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        EngineApiClient::<AlpenEngineTypes>::fork_choice_updated_v3(
            &self.ws_client,
            fork_choice_state,
            payload_attributes,
        )
        .await
    }

    async fn get_payload_v4(
        &self,
        payload_id: PayloadId,
    ) -> RpcResult<AlpenExecutionPayloadEnvelopeV4> {
        EngineApiClient::<AlpenEngineTypes>::get_payload_v4(&self.ws_client, payload_id).await
    }

    async fn new_payload_v4(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        execution_requests: RequestsOrHash,
    ) -> RpcResult<alloy_rpc_types::engine::PayloadStatus> {
        EngineApiClient::<AlpenEngineTypes>::new_payload_v4(
            &self.ws_client,
            payload,
            versioned_hashes,
            parent_beacon_block_root,
            execution_requests,
        )
        .await
    }

    async fn block_by_hash(&self, block_hash: BlockHash) -> RpcResult<Option<RpcBlock>> {
        EthApiClient::<
            TransactionRequest,
            Transaction,
            RpcBlock<alloy_rpc_types::Transaction>,
            Receipt,
            Header,
        >::block_by_hash(&self.ws_client, block_hash, false)
        .await
    }
}
