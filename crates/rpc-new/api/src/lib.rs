//! OL RPC API definitions.

use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use strata_identifiers::{AccountId, OLBlockCommitment, OLBlockId, OLTxId};
use strata_rpc_types_new::*;

/// Core OL RPC methods for querying chain state.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strata"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "strata"))]
pub trait OLApi {
    /// Get current chain status (latest, confirmed, finalized).
    #[method(name = "chainStatus")]
    async fn chain_status(&self) -> RpcResult<RpcOLChainStatus>;

    /// Get block commitments for slot range [start_slot, end_slot].
    #[method(name = "getBlockCommitmentsInRange")]
    async fn get_block_commitments_in_range(
        &self,
        start_slot: u64,
        end_slot: u64,
    ) -> RpcResult<Vec<OLBlockCommitment>>;
}

#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strata"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "strata"))]
pub trait OLSequencerApi {
    /// Get update inputs for specified blocks and account.
    #[method(name = "getUpdateInputsForBlocks")]
    async fn get_update_inputs_for_blocks(
        &self,
        account_id: AccountId,
        blocks: Vec<OLBlockId>,
    ) -> RpcResult<Vec<BlockUpdateInputs>>;

    /// Get message payloads for specified blocks and account.
    #[method(name = "getMessagesForBlocks")]
    async fn get_messages_for_blocks(
        &self,
        account_id: AccountId,
        blocks: Vec<OLBlockId>,
    ) -> RpcResult<Vec<BlockMessages>>;

    /// Submit transaction to mempool. Returns immediately with tx ID.
    #[method(name = "submitTransaction")]
    async fn submit_transaction(&self, tx: RpcOLTransaction) -> RpcResult<OLTxId>;
}
