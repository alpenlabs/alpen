//! OL RPC API definitions.

mod types;

use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use strata_identifiers::{OLBlockCommitment, OLBlockId, OLTxId};
pub use types::*;

/// Core OL RPC methods for querying chain state.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "ol"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "ol"))]
pub trait OlApi {
    /// Get current chain status (latest, confirmed, finalized).
    #[method(name = "chainStatus")]
    async fn chain_status(&self) -> RpcResult<RpcOlChainStatus>;

    /// Get block commitments for slot range [start_slot, end_slot].
    #[method(name = "blockCommitmentsInRange")]
    async fn block_commitments_in_range(
        &self,
        start_slot: u64,
        end_slot: u64,
    ) -> RpcResult<Vec<OLBlockCommitment>>;
}

/// OL sequencer-specific RPC methods.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "ol"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "ol"))]
pub trait OlSequencerApi {
    /// Get message payloads for a range of blocks.
    #[method(name = "inputsForBlockRange")]
    async fn inputs_for_block_range(
        &self,
        account_id: RpcAccountId,
        block_ids: Vec<OLBlockId>,
    ) -> RpcResult<Vec<BlockMessages>>;

    /// Get update inputs for specified blocks and account.
    #[method(name = "updateInputsForBlocks")]
    async fn update_inputs_for_blocks(
        &self,
        account_id: RpcAccountId,
        blocks: Vec<OLBlockId>,
    ) -> RpcResult<Vec<BlockUpdateInputs>>;

    /// Get message payloads for specified blocks and account.
    #[method(name = "messagesForBlocks")]
    async fn messages_for_blocks(
        &self,
        account_id: RpcAccountId,
        blocks: Vec<OLBlockId>,
    ) -> RpcResult<Vec<BlockMessages>>;

    /// Submit transaction to mempool. Returns immediately with tx ID.
    #[method(name = "submitTransaction")]
    async fn submit_transaction(&self, tx: RpcOlTransaction) -> RpcResult<OLTxId>;
}
