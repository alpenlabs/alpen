use std::sync::Arc;

use alloy_eips::eip7685::RequestsOrHash;
use alloy_rpc_types::{
    engine::{ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, JwtSecret, PayloadId},
    eth::{Block as RpcBlock, Header, Transaction},
    TransactionRequest,
};
use alpen_reth_node::{AlpenEngineTypes, AlpenExecutionPayloadEnvelopeV4, AlpenPayloadAttributes};
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
#[cfg(test)]
use mockall::automock;
use reth_primitives::Receipt;
use reth_rpc_api::{EngineApiClient, EthApiClient};
use revm_primitives::alloy_primitives::{BlockHash, B256};

fn http_client(http_url: &str, _secret: JwtSecret) -> HttpClient {
    // TODO: Implement proper JWT authentication when middleware is compatible
    // For now, use basic HTTP client until reth-rpc-layer works with jsonrpsee 0.25.1
    HttpClientBuilder::default()
        .build(http_url)
        .expect("Failed to create http client")
}

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
    client: Arc<HttpClient>,
}

impl EngineRpcClient {
    pub fn from_url_secret(http_url: &str, secret: JwtSecret) -> Self {
        EngineRpcClient {
            client: Arc::new(http_client(http_url, secret)),
        }
    }

    pub fn inner(&self) -> &HttpClient {
        &self.client
    }
}

impl EngineRpc for EngineRpcClient {
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<AlpenPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        <HttpClient as EngineApiClient<AlpenEngineTypes>>::fork_choice_updated_v3(&self.client, fork_choice_state, payload_attributes).await
    }

    async fn get_payload_v4(
        &self,
        payload_id: PayloadId,
    ) -> RpcResult<AlpenExecutionPayloadEnvelopeV4> {
        <HttpClient as EngineApiClient<AlpenEngineTypes>>::get_payload_v4(&self.client, payload_id).await
    }

    async fn new_payload_v4(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        execution_requests: RequestsOrHash,
    ) -> RpcResult<alloy_rpc_types::engine::PayloadStatus> {
        <HttpClient as EngineApiClient<AlpenEngineTypes>>::new_payload_v4(
            &self.client,
            payload,
            versioned_hashes,
            parent_beacon_block_root,
            execution_requests,
        )
        .await
    }

    async fn block_by_hash(&self, block_hash: BlockHash) -> RpcResult<Option<RpcBlock>> {
        <HttpClient as EthApiClient<
            TransactionRequest,
            Transaction,
            RpcBlock<alloy_rpc_types::Transaction>,
            Receipt,
            Header,
        >>::block_by_hash(&self.client, block_hash, false)
        .await
    }
}
