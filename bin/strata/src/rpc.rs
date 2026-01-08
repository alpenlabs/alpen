//! OL RPC server implementation.

use std::{fmt::Display, sync::Arc};

use async_trait::async_trait;
use jsonrpsee::{
    core::RpcResult,
    types::{
        ErrorObjectOwned,
        error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    },
};
use strata_identifiers::{AccountId, Epoch, EpochCommitment, OLBlockCommitment, OLBlockId, OLTxId};
use strata_ledger_types::{
    IAccountState, ISnarkAccountState, ISnarkAccountStateExt, IStateAccessor,
};
use strata_ol_mempool::{MempoolHandle, OLMempoolError, OLMempoolTransaction};
use strata_rpc_api_new::OLClientRpcServer;
use strata_rpc_types_new::{
    RpcAccountBlockSummary, RpcAccountEpochSummary, RpcOLChainStatus, RpcOLTransaction,
};
use strata_snark_acct_types::ProofState;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tracing::error;

/// Custom error code for mempool capacity-related errors.
const MEMPOOL_CAPACITY_ERROR_CODE: i32 = -32001;

/// Creates an RPC error for database failures.
fn db_error(e: impl Display) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        INTERNAL_ERROR_CODE,
        format!("Database error: {e}"),
        None::<()>,
    )
}

/// Creates an RPC error for resource not found.
fn not_found_error(msg: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, msg.into(), None::<()>)
}

/// Creates an RPC error for internal failures.
fn internal_error(msg: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, msg.into(), None::<()>)
}

/// Creates an RPC error for invalid parameters.
fn invalid_params_error(msg: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, msg.into(), None::<()>)
}

/// OL RPC server implementation.
pub(crate) struct OLRpcServer {
    /// Storage backend.
    storage: Arc<NodeStorage>,

    /// Status channel.
    status_channel: Arc<StatusChannel>,

    /// Mempool handle for transaction submission.
    mempool_handle: MempoolHandle,
}

impl OLRpcServer {
    /// Creates a new [`OLRpcServer`].
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
        mempool_handle: MempoolHandle,
    ) -> Self {
        Self {
            storage,
            status_channel,
            mempool_handle,
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
                db_error(e)
            })?;

        // For now, use the first commitment if available
        // TODO: This should be more sophisticated - we might need to determine which commitment
        // corresponds to the canonical chain
        let epoch_commitment = commitments.first().ok_or_else(|| {
            not_found_error(format!("No epoch commitment found for epoch {epoch}"))
        })?;

        // Get OL state at the terminal block using the epoch commitment directly
        // (EpochCommitment already contains the terminal slot and block ID)
        let terminal_commitment = epoch_commitment.to_block_commitment();
        let ol_state = self
            .storage
            .ol_state()
            .get_toplevel_ol_state_async(terminal_commitment)
            .await
            .map_err(|e| {
                error!(?e, %terminal_commitment, "Failed to get OL state");
                db_error(e)
            })?
            .ok_or_else(|| {
                not_found_error(format!(
                    "No OL state found for terminal block {terminal_commitment}"
                ))
            })?;

        // Extract account state
        let account_state = ol_state
            .get_account_state(account_id)
            .map_err(|e| {
                error!(?e, %account_id, "Failed to get account state");
                internal_error(format!("Account error: {e}"))
            })?
            .ok_or_else(|| not_found_error(format!("Account {account_id} not found")))?;

        // Extract snark-specific data if applicable
        let (next_seq_no, proof_state) = match account_state.as_snark_account() {
            Ok(snark_state) => {
                let seqno: u64 = *snark_state.seqno().inner();
                let inner_state = snark_state.inner_state_root();
                let next_inbox_idx = snark_state.get_next_inbox_msg_idx();
                (seqno, ProofState::new(inner_state, next_inbox_idx))
            }
            Err(_) => {
                // Non-snark account - return default values
                (0, ProofState::new([0u8; 32].into(), 0))
            }
        };

        // Get previous epoch commitment if available
        let prev_epoch_commitment = if epoch > 0 {
            let prev_commitments = self
                .storage
                .checkpoint()
                .get_epoch_commitments_at((epoch - 1) as u64)
                .await
                .ok()
                .and_then(|c| c.first().copied());
            prev_commitments.unwrap_or_else(EpochCommitment::null)
        } else {
            EpochCommitment::null()
        };

        Ok(RpcAccountEpochSummary::new(
            *epoch_commitment,
            prev_epoch_commitment,
            account_state.balance().to_sat(),
            next_seq_no,
            proof_state,
            vec![], // extra_data - not tracked per-epoch currently
            vec![], // processed_msgs - requires additional tracking infrastructure
        ))
    }

    async fn chain_status(&self) -> RpcResult<RpcOLChainStatus> {
        let chain_sync_status = self
            .status_channel
            .get_chain_sync_status()
            .ok_or_else(|| internal_error("Chain sync status not available"))?;

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
            return Err(invalid_params_error("start_slot must be <= end_slot"));
        }

        // Walk backwards from end_slot via parent references to ensure blocks are chained.
        // This guarantees all returned blocks are from the same chain, not different forks.
        let mut chain_blocks: Vec<(u64, OLBlockId)> = Vec::new();

        // Get the starting block at end_slot
        let end_block_ids = self
            .storage
            .ol_block()
            .get_blocks_at_height_async(end_slot)
            .await
            .map_err(|e| {
                error!(?e, slot = end_slot, "Failed to get blocks at end_slot");
                db_error(e)
            })?;

        let Some(current_block_id) = end_block_ids.first().copied() else {
            // No block at end_slot, return empty
            return Ok(Vec::new());
        };

        // Walk backwards from end_slot to start_slot following parent references
        let mut current_id = current_block_id;

        loop {
            // Get the block data to access parent
            let block = self
                .storage
                .ol_block()
                .get_block_data_async(current_id)
                .await
                .map_err(|e| {
                    error!(?e, %current_id, "Failed to get block data");
                    db_error(e)
                })?
                .ok_or_else(|| {
                    not_found_error(format!("Block {current_id} not found in database"))
                })?;

            let header = block.header();
            let current_slot = header.slot();

            // Add this block if it's within our range
            if current_slot >= start_slot && current_slot <= end_slot {
                chain_blocks.push((current_slot, current_id));
            }

            // Stop if we've reached or passed start_slot
            if current_slot <= start_slot {
                break;
            }

            // Move to parent block
            current_id = *header.parent_blkid();
        }

        // Reverse to get ascending slot order
        chain_blocks.reverse();

        // Now build summaries for each block in the chain
        let mut summaries = Vec::with_capacity(chain_blocks.len());

        for (slot, block_id) in chain_blocks {
            let block_commitment = OLBlockCommitment::new(slot, block_id);

            // Get OL state at this block
            let ol_state = self
                .storage
                .ol_state()
                .get_toplevel_ol_state_async(block_commitment)
                .await
                .map_err(|e| {
                    error!(?e, %block_commitment, "Failed to get OL state");
                    db_error(e)
                })?;

            let Some(ol_state) = ol_state else {
                continue; // Skip if state not available
            };

            // Get account state
            let account_state = ol_state.get_account_state(account_id).map_err(|e| {
                error!(?e, %account_id, slot, "Failed to get account state");
                internal_error(format!("Account error: {e}"))
            })?;

            let Some(account_state) = account_state else {
                continue; // Account not found at this slot
            };

            // Extract snark-specific data if applicable
            let (next_seq_no, next_inbox_msg_idx) = match account_state.as_snark_account() {
                Ok(snark_state) => {
                    let seqno: u64 = *snark_state.seqno().inner();
                    let next_inbox_idx = snark_state.get_next_inbox_msg_idx();
                    (seqno, next_inbox_idx)
                }
                Err(_) => (0, 0),
            };

            summaries.push(RpcAccountBlockSummary::new(
                account_id,
                block_commitment,
                account_state.balance(),
                next_seq_no,
                vec![], // updates - requires write batch analysis
                vec![], // new_inbox_messages - requires write batch analysis
                next_inbox_msg_idx,
            ));
        }

        Ok(summaries)
    }

    async fn submit_transaction(&self, tx: RpcOLTransaction) -> RpcResult<OLTxId> {
        // Convert RPC transaction to mempool transaction
        let mempool_tx: OLMempoolTransaction = tx
            .try_into()
            .map_err(|e| invalid_params_error(format!("Invalid transaction: {e}")))?;

        // Submit to mempool
        let txid = self
            .mempool_handle
            .submit_transaction(mempool_tx)
            .await
            .map_err(map_mempool_error_to_rpc)?;

        Ok(txid)
    }
}

/// Maps mempool errors to RPC errors with appropriate error codes.
fn map_mempool_error_to_rpc(err: OLMempoolError) -> ErrorObjectOwned {
    match &err {
        // Capacity-related errors
        OLMempoolError::MempoolFull { .. } | OLMempoolError::MempoolByteLimitExceeded { .. } => {
            ErrorObjectOwned::owned(MEMPOOL_CAPACITY_ERROR_CODE, err.to_string(), None::<()>)
        }
        // Validation errors that are user's fault
        OLMempoolError::AccountDoesNotExist { .. }
        | OLMempoolError::AccountTypeMismatch { .. }
        | OLMempoolError::TransactionTooLarge { .. }
        | OLMempoolError::TransactionExpired { .. }
        | OLMempoolError::TransactionNotMature { .. }
        | OLMempoolError::UsedSequenceNumber { .. }
        | OLMempoolError::SequenceNumberGap { .. } => invalid_params_error(err.to_string()),
        // Internal errors
        OLMempoolError::AccountStateAccess(_)
        | OLMempoolError::TransactionNotFound(_)
        | OLMempoolError::Database(_)
        | OLMempoolError::Serialization(_)
        | OLMempoolError::ServiceClosed(_)
        | OLMempoolError::StateProvider(_) => {
            error!(?err, "Internal mempool error");
            internal_error(err.to_string())
        }
    }
}
