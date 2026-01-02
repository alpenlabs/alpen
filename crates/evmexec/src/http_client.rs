use std::sync::Arc;

use alloy_eips::eip7685::RequestsOrHash;
use alloy_rpc_types::{
    engine::{
        ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, JwtSecret, PayloadId, PayloadStatus,
    },
    eth::{Block as RpcBlock, Header, Transaction, TransactionRequest},
};
use alpen_reth_node::{AlpenEngineTypes, AlpenExecutionPayloadEnvelopeV4, AlpenPayloadAttributes};
use jsonrpsee::{
    core::{middleware::layer::RpcLogger, ClientError},
    http_client::{transport::HttpBackend, HttpClient, HttpClientBuilder},
};
use jsonrpsee_http_client::RpcService;
#[cfg(test)]
use mockall::automock;
use reth_primitives::{Receipt, TransactionSigned};
use reth_rpc_api::{EngineApiClient, EthApiClient};
use reth_rpc_layer::{AuthClientLayer, AuthClientService};
use revm_primitives::alloy_primitives::{BlockHash, B256};

type RpcResult<T> = Result<T, ClientError>;

#[allow(
    async_fn_in_trait,
    clippy::allow_attributes,
    reason = "we don't want async_trait"
)]
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
    ) -> RpcResult<PayloadStatus>;

    async fn block_by_hash(&self, block_hash: BlockHash) -> RpcResult<Option<RpcBlock>>;
}

#[derive(Debug, Clone)]
pub struct EngineRpcClient {
    client: Arc<EngineHttpClient>,
}

pub(crate) type EngineHttpClient =
    HttpClient<RpcLogger<RpcService<AuthClientService<HttpBackend>>>>;

impl EngineRpcClient {
    pub fn from_url_secret(http_url: &str, secret: JwtSecret) -> Self {
        EngineRpcClient {
            client: Arc::new(http_client(http_url, secret)),
        }
    }
}

impl EngineRpc for EngineRpcClient {
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<AlpenPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        <EngineHttpClient as EngineApiClient<AlpenEngineTypes>>::fork_choice_updated_v3(
            &self.client,
            fork_choice_state,
            payload_attributes,
        )
        .await
    }

    async fn get_payload_v4(
        &self,
        payload_id: PayloadId,
    ) -> RpcResult<AlpenExecutionPayloadEnvelopeV4> {
        <EngineHttpClient as EngineApiClient<AlpenEngineTypes>>::get_payload_v4(
            &self.client,
            payload_id,
        )
        .await
    }

    async fn new_payload_v4(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        execution_requests: RequestsOrHash,
    ) -> RpcResult<PayloadStatus> {
        <EngineHttpClient as EngineApiClient<AlpenEngineTypes>>::new_payload_v4(
            &self.client,
            payload,
            versioned_hashes,
            parent_beacon_block_root,
            execution_requests,
        )
        .await
    }

    async fn block_by_hash(&self, block_hash: BlockHash) -> RpcResult<Option<RpcBlock>> {
        <EngineHttpClient as EthApiClient<
            TransactionRequest,
            Transaction,
            RpcBlock<alloy_rpc_types::Transaction>,
            Receipt,
            Header,
            TransactionSigned,
        >>::block_by_hash(&self.client, block_hash, false)
        .await
    }
}

fn http_client(http_url: &str, secret: JwtSecret) -> EngineHttpClient {
    let middleware = tower::ServiceBuilder::new().layer(AuthClientLayer::new(secret));

    HttpClientBuilder::default()
        .set_http_middleware(middleware)
        .build(http_url)
        .expect("Failed to create http client")
}
