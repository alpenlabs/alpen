//! OL RPC API definitions.

use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use strata_identifiers::{AccountId, OLTxId};
use strata_primitives::proof::Epoch;
use strata_rpc_types_new::*;

/// Common OL RPC methods that are served by all kinds of nodes(DA, block executing).
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strata"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "strata"))]
pub trait OLClientRpc {
    /// Get an account's epoch summary for a given epoch.
    #[method(name = "getAccountEpochSummary")]
    async fn get_acct_epoch_summary(
        &self,
        account_id: AccountId,
        epoch: Epoch,
    ) -> RpcResult<RpcAccountEpochSummary>;

    /// Get current chain status (latest, confirmed, finalized).
    #[method(name = "getChainStatus")]
    async fn chain_status(&self) -> RpcResult<RpcOLChainStatus>;

    /// Get summaries associated with an account for given blocks.
    #[method(name = "getBlocksSummaries")]
    async fn get_blocks_summaries(
        &self,
        account_id: AccountId,
        start_slot: u64,
        end_slot: u64,
    ) -> RpcResult<Vec<RpcAccountBlockSummary>>;

    /// Submit transaction to the node. Returns immediately with tx ID.
    #[method(name = "submitTransaction")]
    async fn submit_transaction(&self, tx: RpcOLTransaction) -> RpcResult<OLTxId>;
}
