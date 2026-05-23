//! Alpen EE RPC API definitions.

use alloy_primitives::B256;
pub use alpen_ee_rpc_types::{
    BlockStatus, BlockStatusResponse, ProofPipelineBatch, ProofPipelineBatchStatus,
    ProofPipelineChunk, ProofPipelineChunkStatus, ProofPipelineStatusResponse,
};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};

/// RPC methods exposed by Alpen EE nodes.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "alpen"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "alpen"))]
pub trait AlpenEeRpc {
    /// Returns the L1 finalization status for an EE block.
    #[method(name = "getBlockStatus")]
    async fn get_block_status(&self, block_hash: B256) -> RpcResult<BlockStatusResponse>;
}

/// Test and local-diagnostic RPC methods for Alpen EE proof pipeline state.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "alpen"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "alpen"))]
pub trait AlpenEeProofPipelineRpc {
    /// Returns storage-backed EE proof pipeline progress.
    #[method(name = "getProofPipelineStatus")]
    async fn get_proof_pipeline_status(&self) -> RpcResult<ProofPipelineStatusResponse>;
}
