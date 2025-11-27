//! OL RPC API definitions.

use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use strata_identifiers::{AccountId, OLBlockCommitment, OLBlockId, OLTxId};
use strata_primitives::proof::Epoch;
use strata_rpc_types_new::*;

/// Common OL RPC methods that are served by all kinds of nodes(DA, block executing).
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strata"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "strata"))]
pub trait OLCommonApi {
    /// Get an account's epoch summary for a given epoch.
    #[method(name = "getAccountEpochSummary")]
    async fn get_acct_epoch_summary(
        &self,
        account_id: AccountId,
        epoch: Epoch,
    ) -> RpcResult<RpcAccountEpochSummary>;
}

/// Api methods served by all block executing nodes.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strata"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "strata"))]
pub trait OLFullNodeApi {
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

    /// Get summaries associated with an account for given blocks.
    #[method(name = "getBlocksSummaries")]
    async fn get_blocks_data(
        &self,
        account_id: AccountId,
        blocks: Vec<OLBlockId>,
    ) -> RpcResult<Vec<RpcAccountBlockSummary>>;

    /// Submit transaction to the node. Returns immediately with tx ID.
    #[method(name = "submitTransaction")]
    async fn submit_transaction(&self, tx: RpcOLTransaction) -> RpcResult<OLTxId>;
}
