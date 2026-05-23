//! Alpen EE RPC API definitions.

use alloy_primitives::B256;
pub use alpen_ee_rpc_types::{BlockStatus, BlockStatusResponse};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};

/// RPC methods exposed by Alpen EE nodes.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "alpen"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "alpen"))]
pub trait AlpenEeRpc {
    /// Returns the L1 finalization status for an EE block.
    #[method(name = "getBlockStatus")]
    async fn get_block_status(&self, block_hash: B256) -> RpcResult<BlockStatusResponse>;
}
