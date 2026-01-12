//! OL RPC API definitions.

use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use strata_identifiers::{AccountId, Epoch, OLTxId};
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

    /// Get account-specific summaries for blocks in a slot range.
    ///
    /// Returns the account's state (balance, sequence number, inbox position) at each block
    /// in the range `[start_slot, end_slot]`. This is useful for clients that need to track
    /// how an account's state evolved over a series of blocks, such as snark account provers
    /// that need to know inbox messages and state transitions.
    ///
    /// Results are returned in ascending slot order. Only blocks on the canonical chain
    /// are included; the implementation walks parent references to ensure chain continuity.
    #[method(name = "getBlocksSummaries")]
    async fn get_blocks_summaries(
        &self,
        account_id: AccountId,
        start_slot: u64,
        end_slot: u64,
    ) -> RpcResult<Vec<RpcAccountBlockSummary>>;

    /// Get snark account state of an account at a specified block.
    #[method(name = "getSnarkAccountState")]
    async fn get_snark_account_state(
        &self,
        account_id: AccountId,
        block_or_tag: OLBlockOrTag,
    ) -> RpcResult<Option<RpcSnarkAccountState>>;

    /// Submit transaction to the node. Returns immediately with tx ID.
    #[method(name = "submitTransaction")]
    async fn submit_transaction(&self, tx: RpcOLTransaction) -> RpcResult<OLTxId>;
}
