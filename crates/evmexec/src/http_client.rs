use std::sync::Arc;

use alloy_rpc_types::engine::{
    ExecutionPayloadBodiesV1, ExecutionPayloadInputV2, ForkchoiceState, ForkchoiceUpdated,
    JwtSecret, PayloadId,
};
use alpen_reth_node::{
    StrataEngineTypes, StrataExecutionPayloadEnvelopeV2, StrataPayloadAttributes,
};
use jsonrpsee::http_client::{transport::HttpBackend, HttpClient, HttpClientBuilder};
#[cfg(test)]
use mockall::automock;
use reth_primitives::{Block, SealedBlock, TransactionSigned};
use reth_rpc_api::{EngineApiClient, EthApiClient};
use reth_rpc_layer::{AuthClientLayer, AuthClientService};
use revm_primitives::alloy_primitives::BlockHash;
use strata_common::metrics::{RPC_CALLS_TOTAL, RPC_CALL_DURATION, RPC_PAYLOAD_BYTES};

fn http_client(http_url: &str, secret: JwtSecret) -> HttpClient<AuthClientService<HttpBackend>> {
    let middleware = tower::ServiceBuilder::new().layer(AuthClientLayer::new(secret));

    HttpClientBuilder::default()
        .set_http_middleware(middleware)
        .build(http_url)
        .expect("Failed to create http client")
}

type RpcResult<T> = Result<T, jsonrpsee::core::ClientError>;

#[allow(async_fn_in_trait)]
#[cfg_attr(test, automock)]
pub trait EngineRpc {
    async fn fork_choice_updated_v2(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<StrataPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated>;

    async fn get_payload_v2(
        &self,
        payload_id: PayloadId,
    ) -> RpcResult<StrataExecutionPayloadEnvelopeV2>;

    async fn new_payload_v2(
        &self,
        payload: ExecutionPayloadInputV2,
    ) -> RpcResult<alloy_rpc_types::engine::PayloadStatus>;

    async fn get_payload_bodies_by_hash_v1(
        &self,
        block_hashes: Vec<BlockHash>,
    ) -> RpcResult<ExecutionPayloadBodiesV1>;

    async fn block_by_hash(&self, block_hash: BlockHash) -> RpcResult<Option<Block>>;
}

#[derive(Debug, Clone)]
pub struct EngineRpcClient {
    client: Arc<HttpClient<AuthClientService<HttpBackend>>>,
}

impl EngineRpcClient {
    pub fn from_url_secret(http_url: &str, secret: JwtSecret) -> Self {
        EngineRpcClient {
            client: Arc::new(http_client(http_url, secret)),
        }
    }

    pub fn inner(&self) -> &HttpClient<AuthClientService<HttpBackend>> {
        &self.client
    }
}

impl EngineRpc for EngineRpcClient {
    async fn fork_choice_updated_v2(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<StrataPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated> {
        let start = std::time::Instant::now();

        // Estimate request payload size
        let request_size = std::mem::size_of_val(&fork_choice_state)
            + payload_attributes.as_ref().map(|pa| std::mem::size_of_val(pa)).unwrap_or(0);
        RPC_PAYLOAD_BYTES
            .with_label_values(&["fork_choice_updated_v2", "request", "execution_engine"])
            .observe(request_size as f64);

        let result = <HttpClient<AuthClientService<HttpBackend>> as EngineApiClient<StrataEngineTypes>>::fork_choice_updated_v2(&self.client, fork_choice_state, payload_attributes).await;

        let duration = start.elapsed().as_secs_f64();
        RPC_CALL_DURATION
            .with_label_values(&["fork_choice_updated_v2", "execution_engine"])
            .observe(duration);

        match &result {
            Ok(response) => {
                RPC_CALLS_TOTAL
                    .with_label_values(&["fork_choice_updated_v2", "execution_engine", "success"])
                    .inc();
                let response_size = std::mem::size_of_val(response);
                RPC_PAYLOAD_BYTES
                    .with_label_values(&["fork_choice_updated_v2", "response", "execution_engine"])
                    .observe(response_size as f64);
            }
            Err(_) => {
                RPC_CALLS_TOTAL
                    .with_label_values(&["fork_choice_updated_v2", "execution_engine", "failed"])
                    .inc();
            }
        }

        result
    }

    async fn get_payload_v2(
        &self,
        payload_id: PayloadId,
    ) -> RpcResult<StrataExecutionPayloadEnvelopeV2> {
        <HttpClient<AuthClientService<HttpBackend>> as EngineApiClient<StrataEngineTypes>>::get_payload_v2(&self.client, payload_id).await
    }

    async fn new_payload_v2(
        &self,
        payload: ExecutionPayloadInputV2,
    ) -> RpcResult<alloy_rpc_types::engine::PayloadStatus> {
        let start = std::time::Instant::now();

        // Track payload size
        let payload_size = std::mem::size_of_val(&payload);
        RPC_PAYLOAD_BYTES
            .with_label_values(&["new_payload_v2", "request", "execution_engine"])
            .observe(payload_size as f64);

        let result = <HttpClient<AuthClientService<HttpBackend>> as EngineApiClient<StrataEngineTypes>>::new_payload_v2(&self.client, payload).await;

        let duration = start.elapsed().as_secs_f64();
        RPC_CALL_DURATION
            .with_label_values(&["new_payload_v2", "execution_engine"])
            .observe(duration);

        match &result {
            Ok(response) => {
                RPC_CALLS_TOTAL
                    .with_label_values(&["new_payload_v2", "execution_engine", "success"])
                    .inc();
                let response_size = std::mem::size_of_val(response);
                RPC_PAYLOAD_BYTES
                    .with_label_values(&["new_payload_v2", "response", "execution_engine"])
                    .observe(response_size as f64);
            }
            Err(_) => {
                RPC_CALLS_TOTAL
                    .with_label_values(&["new_payload_v2", "execution_engine", "failed"])
                    .inc();
            }
        }

        result
    }

    async fn get_payload_bodies_by_hash_v1(
        &self,
        block_hashes: Vec<BlockHash>,
    ) -> RpcResult<ExecutionPayloadBodiesV1> {
        <HttpClient<AuthClientService<HttpBackend>> as EngineApiClient<StrataEngineTypes>>::get_payload_bodies_by_hash_v1(&self.client, block_hashes).await
    }

    async fn block_by_hash(&self, block_hash: BlockHash) -> RpcResult<Option<Block>> {
        let block = <HttpClient<AuthClientService<HttpBackend>> as EthApiClient<
            alloy_network::AnyRpcTransaction,
            alloy_network::AnyRpcBlock,
            alloy_network::AnyTransactionReceipt,
            alloy_network::AnyRpcBlock,
        >>::block_by_hash(&self.client, block_hash, true)
        .await?;

        Ok(block.map(|b| {
            let sealed_block: SealedBlock = b.try_into().unwrap();
            Block::<TransactionSigned>::from(sealed_block)
        }))
    }
}
