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
use ssz::Encode;
use strata_identifiers::{AccountId, Epoch, EpochCommitment, OLBlockCommitment, OLBlockId, OLTxId};
use strata_ledger_types::{
    IAccountState, ISnarkAccountState, ISnarkAccountStateExt, IStateAccessor,
};
use strata_ol_chain_types_new::OLBlock;
use strata_ol_mempool::{MempoolHandle, OLMempoolError, OLMempoolTransaction};
use strata_primitives::HexBytes;
use strata_rpc_api_new::{OLClientRpcServer, OLFullNodeRpcServer};
use strata_rpc_types_new::{
    OLBlockOrTag, RpcAccountBlockSummary, RpcAccountEpochSummary, RpcBlockRangeEntry,
    RpcOLChainStatus, RpcOLTransaction, RpcSnarkAccountState,
};
use strata_snark_acct_types::ProofState;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tracing::error;

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

    async fn get_canonical_block_at_height(&self, height: u64) -> RpcResult<Option<OLBlockId>> {
        let blkid = self
            .storage
            .ol_block()
            .get_blocks_at_height_async(height)
            .await
            .map_err(db_error)?
            .first() // TODO: Assumes the canonical is the first one, but need to define it
            .copied();
        Ok(blkid)
    }

    async fn get_block(&self, blkid: OLBlockId) -> RpcResult<OLBlock> {
        let blk = self
            .storage
            .ol_block()
            .get_block_data_async(blkid)
            .await
            .map_err(db_error)?
            .ok_or(not_found_error(format!("block not found: {blkid}")))?;
        Ok(blk)
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

        // Extract snark-specific data if applicable.
        // For non-snark accounts, these fields are zeroed since seqno and proof state
        // concepts don't apply to them.
        let (next_seq_no, proof_state) = match account_state.as_snark_account() {
            Ok(snark_state) => {
                let seqno: u64 = *snark_state.seqno().inner();
                let inner_state = snark_state.inner_state_root();
                let next_inbox_idx = snark_state.get_next_inbox_msg_idx();
                (seqno, ProofState::new(inner_state, next_inbox_idx))
            }
            Err(_) => (0, ProofState::new([0u8; 32].into(), 0)), // Non-snark account
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

        // Get finalized slot - blocks at or before this are guaranteed to be on canonical chain
        let finalized_slot = self
            .status_channel
            .get_chain_sync_status()
            .map(|css| css.finalized_epoch.last_slot())
            .unwrap_or(0);

        // Walk backwards from end_slot via parent references to ensure blocks are chained.
        // This guarantees all returned blocks are from the same chain, not different forks.
        // Once we reach a finalized block, we can fetch remaining blocks directly by slot.
        let mut chain_blocks: Vec<(u64, OLBlockId)> = Vec::new();

        // Get the block at end_slot (we'll walk backwards from here)
        let end_block_id = self.get_canonical_block_at_height(end_slot).await?;

        let Some(current_block_id) = end_block_id else {
            // No block at end_slot, return empty
            return Ok(Vec::new());
        };

        // Walk backwards from end_slot to start_slot following parent references
        let mut current_id = current_block_id;

        loop {
            // Get the block data to access parent
            let block = self.get_block(current_id).await?;

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

            // Optimization: if we've reached a finalized slot, fetch remaining blocks directly
            // by slot instead of walking parent references (finalized blocks have a single chain)
            if current_slot <= finalized_slot {
                // Fetch remaining blocks directly by slot
                for slot in (start_slot..current_slot).rev() {
                    let blkid = self.get_canonical_block_at_height(slot).await?;
                    if let Some(blkid) = blkid {
                        chain_blocks.push((slot, blkid));
                    }
                }
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

            // Extract snark-specific data if applicable.
            // For non-snark accounts, these fields are zeroed since seqno and inbox
            // concepts don't apply to them.
            //
            // Note: Unlike `get_acct_epoch_summary` which returns a full `ProofState`,
            // `RpcAccountBlockSummary` only has `next_inbox_msg_idx` as a separate field
            // (no `inner_state`). This is by design - per-block summaries focus on
            // tracking changes rather than full proof state.
            let (next_seq_no, next_inbox_msg_idx) = match account_state.as_snark_account() {
                Ok(snark_state) => {
                    let seqno: u64 = *snark_state.seqno().inner();
                    let next_inbox_idx = snark_state.get_next_inbox_msg_idx();
                    (seqno, next_inbox_idx)
                }
                Err(_) => (0, 0), // Non-snark account
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

    async fn get_snark_account_state(
        &self,
        account_id: AccountId,
        block_or_tag: OLBlockOrTag,
    ) -> RpcResult<Option<RpcSnarkAccountState>> {
        // Resolve block_or_tag to a block commitment
        let block_commitment = match block_or_tag {
            OLBlockOrTag::Latest => {
                let chain_sync_status = self
                    .status_channel
                    .get_chain_sync_status()
                    .ok_or_else(|| internal_error("Chain sync status not available"))?;
                chain_sync_status.tip
            }
            OLBlockOrTag::Confirmed => {
                let chain_sync_status = self
                    .status_channel
                    .get_chain_sync_status()
                    .ok_or_else(|| internal_error("Chain sync status not available"))?;
                chain_sync_status.prev_epoch.to_block_commitment()
            }
            OLBlockOrTag::Finalized => {
                let chain_sync_status = self
                    .status_channel
                    .get_chain_sync_status()
                    .ok_or_else(|| internal_error("Chain sync status not available"))?;
                chain_sync_status.finalized_epoch.to_block_commitment()
            }
            OLBlockOrTag::OLBlockId(block_id) => {
                let block = self
                    .storage
                    .ol_block()
                    .get_block_data_async(block_id)
                    .await
                    .map_err(|e| {
                        error!(?e, %block_id, "Failed to get block data");
                        db_error(e)
                    })?
                    .ok_or_else(|| not_found_error(format!("Block {block_id} not found")))?;
                OLBlockCommitment::new(block.header().slot(), block_id)
            }
            OLBlockOrTag::Slot(slot) => {
                let block_ids = self
                    .storage
                    .ol_block()
                    .get_blocks_at_height_async(slot)
                    .await
                    .map_err(|e| {
                        error!(?e, slot, "Failed to get blocks at slot");
                        db_error(e)
                    })?;
                let block_id = block_ids
                    .first()
                    .copied()
                    .ok_or_else(|| not_found_error(format!("No block found at slot {slot}")))?;
                OLBlockCommitment::new(slot, block_id)
            }
        };

        // Get OL state at the resolved block
        let ol_state = self
            .storage
            .ol_state()
            .get_toplevel_ol_state_async(block_commitment)
            .await
            .map_err(|e| {
                error!(?e, %block_commitment, "Failed to get OL state");
                db_error(e)
            })?
            .ok_or_else(|| {
                not_found_error(format!("No OL state found for block {block_commitment}"))
            })?;

        // Get account state
        let account_state = match ol_state.get_account_state(account_id) {
            Ok(Some(state)) => state,
            Ok(None) => return Ok(None), // Account doesn't exist
            Err(e) => {
                error!(?e, %account_id, "Failed to get account state");
                return Err(internal_error(format!("Account error: {e}")));
            }
        };

        // Try to get snark account state; return None if not a snark account
        match account_state.as_snark_account() {
            Ok(snark_state) => {
                // Manually construct RpcSnarkAccountState from the native state
                // Note: update_vk is not available from NativeSnarkAccountState (it's stored
                // as account metadata, not runtime state), so we return an empty vec for now
                let seq_no: u64 = *snark_state.seqno().inner();
                let inner_state = snark_state.inner_state_root().0.into();
                let next_inbox_msg_idx = snark_state.get_next_inbox_msg_idx();
                let update_vk = vec![].into(); // Not available from native state

                Ok(Some(RpcSnarkAccountState::new(
                    seq_no,
                    inner_state,
                    next_inbox_msg_idx,
                    update_vk,
                )))
            }
            Err(_) => Ok(None), // Not a snark account
        }
    }
}

const MAX_RAW_BLOCKS_RANGE: usize = 5000; // FIXME: make this configurable

#[async_trait]
impl OLFullNodeRpcServer for OLRpcServer {
    async fn get_raw_blocks_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> RpcResult<Vec<RpcBlockRangeEntry>> {
        let block_count = (end_height.saturating_sub(start_height) + 1) as usize;

        if start_height > end_height || block_count > MAX_RAW_BLOCKS_RANGE {
            return Err(invalid_params_error("Invalid block range"));
        }

        let last = self
            .get_canonical_block_at_height(end_height)
            .await?
            .ok_or(not_found_error(format!(
                "No blocks found at slot {end_height}"
            )))?;

        let mut cur_blk = last;
        let mut blocks = Vec::with_capacity(block_count);

        // Fetch blocks in backward order to ensure a valid chain.
        for _ in (start_height..=end_height).rev() {
            let blk = self.get_block(cur_blk).await?;
            cur_blk = blk.header().parent_blkid;
            blocks.push(blk);
        }
        // Reverse back to get chronological sequence.
        blocks.reverse();

        let entries: Vec<_> = blocks.iter().map(Into::into).collect();

        Ok(entries)
    }

    async fn get_raw_block_by_id(&self, block_id: OLBlockId) -> RpcResult<HexBytes> {
        let raw_blk = self
            .get_block(block_id)
            .await
            .map(|b| HexBytes(b.as_ssz_bytes()))?;
        Ok(raw_blk)
    }
}

const MAX_RAW_BLOCKS_RANGE: usize = 5000; // FIXME: make this configurable

#[async_trait]
impl OLFullNodeRpcServer for OLRpcServer {
    async fn get_raw_blocks_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> RpcResult<HexBytes> {
        let block_count = (end_height.saturating_sub(start_height) + 1) as usize;

        if start_height > end_height || block_count > MAX_RAW_BLOCKS_RANGE {
            return Err(invalid_params_error("Invalid block range"));
        }

        let last = self
            .get_canonical_block_at_height(end_height)
            .await?
            .ok_or(not_found_error(format!(
                "No blocks found at slot {end_height}"
            )))?;

        let mut cur_blk = last;
        let mut blocks = Vec::with_capacity(block_count);

        // Fetch blocks in backward order to ensure a valid chain.
        for _ in (start_height..=end_height).rev() {
            let blk = self.get_block(cur_blk).await?;
            cur_blk = blk.header().parent_blkid;
            blocks.push(blk);
        }
        // Reverse back to get chronological sequence.
        blocks.reverse();
        let blks: VariableList<_, MAX_RAW_BLOCKS_RANGE> = VariableList::new(blocks)
            .map_err(|e| internal_error(format!("cannot collect OL blocks: {e}")))?;

        Ok(HexBytes(blks.as_ssz_bytes()))
    }

    async fn get_raw_block_by_id(&self, block_id: OLBlockId) -> RpcResult<HexBytes> {
        let raw_blk = self
            .get_block(block_id)
            .await
            .map(|b| HexBytes(b.as_ssz_bytes()))?;
        Ok(raw_blk)
    }
}

// === RPC Error Helpers ===

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
