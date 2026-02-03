//! OL RPC API definitions.

use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use strata_identifiers::{AccountId, Epoch, OLBlockId, OLTxId};
use strata_ol_rpc_types::*;
use strata_ol_sequencer::BlockCompletionData;
use strata_primitives::{HexBytes, HexBytes64};

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

/// OL RPC methods served by block executing nodes.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strata"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "strata"))]
pub trait OLFullNodeRpc {
    /// Get blocks in range as raw bytes of serialized `Vec<OLBlock>`.
    /// `start_height` and `end_height` are inclusive.
    #[method(name = "getRawBlocksRange")]
    async fn get_raw_blocks_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> RpcResult<Vec<RpcBlockRangeEntry>>;

    /// Get serialized block for a given block id.
    #[method(name = "getRawBlockById")]
    async fn get_raw_block_by_id(&self, block_id: OLBlockId) -> RpcResult<HexBytes>;
}

/// OL RPC methods served by sequencer node for sequencer signer.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "strata"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "strata"))]
pub trait OLSequencerRpc {
    /// Serve duties for sequencer
    #[method(name = "strataadmin_getSequencerDuties")]
    async fn get_sequencer_duties(&self) -> RpcResult<Vec<RpcDuty>>;

    /// Complete block template
    #[method(name = "strataadmin_completeBlockTemplate")]
    async fn complete_block_template(
        &self,
        template_id: OLBlockId,
        completion: BlockCompletionData,
    ) -> RpcResult<OLBlockId>;

    #[method(name = "strataadmin_completeCheckpointSignature")]
    async fn complete_checkpoint_signature(&self, epoch: Epoch, sig: HexBytes64) -> RpcResult<()>;
}
