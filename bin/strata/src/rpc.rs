//! OL RPC server implementation.

use std::sync::Arc;

use async_trait::async_trait;
use jsonrpsee::{
    core::RpcResult,
    types::{
        ErrorObjectOwned,
        error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    },
};
use strata_identifiers::{AccountId, Epoch, OLTxId};
use strata_rpc_api_new::OLClientRpcServer;
use strata_rpc_types_new::{
    RpcAccountBlockSummary, RpcAccountEpochSummary, RpcOLChainStatus, RpcOLTransaction,
};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tracing::{error, warn};

/// OL RPC server implementation.
pub(crate) struct OLRpcServer {
    /// Storage backend.
    storage: Arc<NodeStorage>,

    /// Status channel.
    status_channel: Arc<StatusChannel>,
}

impl OLRpcServer {
    /// Creates a new [`OLRpcServer`].
    pub(crate) fn new(storage: Arc<NodeStorage>, status_channel: Arc<StatusChannel>) -> Self {
        Self {
            storage,
            status_channel,
        }
    }
}

#[async_trait]
impl OLClientRpcServer for OLRpcServer {
    async fn get_acct_epoch_summary(
        &self,
        account_id: AccountId,
        epoch: Epoch,
    ) -> RpcResult<RpcAccountEpochSummary> {
        // Get epoch commitments for the given epoch
        let commitments = self
            .storage
            .checkpoint()
            .get_epoch_commitments_at(epoch as u64)
            .await
            .map_err(|e| {
                error!(?e, ?epoch, "Failed to get epoch commitments");
                ErrorObjectOwned::owned(
                    INTERNAL_ERROR_CODE,
                    format!("Database error: {e}"),
                    None::<()>,
                )
            })?;

        // For now, use the first commitment if available
        // TODO: This should be more sophisticated - we might need to determine which commitment
        // corresponds to the canonical chain
        let epoch_commitment = commitments.first().ok_or_else(|| {
            ErrorObjectOwned::owned(
                INVALID_PARAMS_CODE,
                format!("No epoch commitment found for epoch {epoch}"),
                None::<()>,
            )
        })?;

        // Get the epoch summary
        let epoch_summary = self
            .storage
            .checkpoint()
            .get_epoch_summary(*epoch_commitment)
            .await
            .map_err(|e| {
                error!(?e, %epoch_commitment, "Failed to get epoch summary");
                ErrorObjectOwned::owned(
                    INTERNAL_ERROR_CODE,
                    format!("Database error: {e}"),
                    None::<()>,
                )
            })?
            .ok_or_else(|| {
                ErrorObjectOwned::owned(
                    INVALID_PARAMS_CODE,
                    format!("No epoch summary found for epoch commitment {epoch_commitment}"),
                    None::<()>,
                )
            })?;

        // Get chainstate at the terminal block to extract account-specific data
        let terminal_blkid = epoch_summary.terminal().blkid();
        let chainstate = self
            .storage
            .chainstate()
            .get_slot_write_batch_async(*terminal_blkid)
            .await
            .map_err(|e| {
                error!(?e, %terminal_blkid, "Failed to get chainstate");
                ErrorObjectOwned::owned(
                    INTERNAL_ERROR_CODE,
                    format!("Database error: {e}"),
                    None::<()>,
                )
            })?
            .ok_or_else(|| {
                ErrorObjectOwned::owned(
                    INVALID_PARAMS_CODE,
                    format!("No chainstate found for terminal block {terminal_blkid}"),
                    None::<()>,
                )
            })?;

        // TODO: Access OL account state from execution environment state
        // For now, return placeholder data as OL account state access is not yet implemented
        // The account state is stored in the execution environment's state root (cur_state),
        // but we need a way to access the actual account data from that root.
        let _chainstate = chainstate.into_toplevel();

        Err(ErrorObjectOwned::owned(
            INTERNAL_ERROR_CODE,
            format!(
                "Account state extraction for account {account_id} at epoch {epoch} not implemented"
            ),
            None::<()>,
        ))
    }

    async fn chain_status(&self) -> RpcResult<RpcOLChainStatus> {
        let chain_sync_status = self.status_channel.get_chain_sync_status().ok_or_else(|| {
            ErrorObjectOwned::owned(
                INTERNAL_ERROR_CODE,
                "Chain sync status not available",
                None::<()>,
            )
        })?;

        let latest = chain_sync_status.tip;
        let confirmed = chain_sync_status.prev_epoch;
        let finalized = chain_sync_status.finalized_epoch;

        Ok(RpcOLChainStatus::new(latest, confirmed, finalized))
    }

    async fn get_blocks_summaries(
        &self,
        account_id: AccountId,
        start_slot: u64,
        end_slot: u64,
    ) -> RpcResult<Vec<RpcAccountBlockSummary>> {
        if start_slot > end_slot {
            return Err(ErrorObjectOwned::owned(
                INVALID_PARAMS_CODE,
                "start_slot must be <= end_slot",
                None::<()>,
            ));
        }

        Err(ErrorObjectOwned::owned(
            INTERNAL_ERROR_CODE,
            format!(
                "Account state extraction for account {account_id} not implemented for slots {start_slot}-{end_slot}"
            ),
            None::<()>,
        ))
    }

    async fn submit_transaction(&self, tx: RpcOLTransaction) -> RpcResult<OLTxId> {
        // Log the transaction
        warn!(
            ?tx,
            "Received transaction submission (OL mempool not yet implemented)"
        );

        // TODO: Optionally persist to storage if there's a suitable table
        // For now, just log and return unimplemented error

        Err(ErrorObjectOwned::owned(
            INTERNAL_ERROR_CODE,
            "OL mempool not implemented; transaction was logged but not processed",
            None::<()>,
        ))
    }
}
