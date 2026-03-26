//! Alpen client RPC API definitions.

use alloy_primitives::B256;
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use serde::{Deserialize, Serialize};

/// L1 finalization status of an EE block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockStatus {
    /// Block is not yet covered by any confirmed or finalized checkpoint.
    Pending,

    /// Block is covered by a confirmed OL checkpoint.
    Confirmed,

    /// Block is covered by a finalized OL checkpoint.
    Finalized,
}

/// Response for `strataee_getBlockStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockStatusResponse {
    /// L1 finalization status.
    pub status: BlockStatus,
}

#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strataee"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "strataee"))]
pub trait AlpenClientRpc {
    /// Returns the L1 finalization status for an EE block.
    #[method(name = "getBlockStatus")]
    async fn get_block_status(&self, block_hash: B256) -> RpcResult<BlockStatusResponse>;
}
