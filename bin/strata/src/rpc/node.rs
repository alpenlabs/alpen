//! OL RPC server implementation for a strata node.
use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use ssz::Encode;
use strata_acct_types::MessageEntry;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{
    AccountId, Epoch, EpochCommitment, L1BlockCommitment, L1Height, L2BlockCommitment,
    OLBlockCommitment, OLBlockId, OLTxId,
};
use strata_ledger_types::{IAccountState, ISnarkAccountState};
use strata_ol_chain_types_new::{OLBlock, OLTransaction, TransactionPayload};
use strata_ol_rpc_api::{OLClientRpcServer, OLFullNodeRpcServer};
use strata_ol_rpc_types::{
    OLBlockOrTag, OLRpcProvider, RpcAccountBlockSummary, RpcAccountEpochSummary, RpcBlockEntry,
    RpcBlockHeaderEntry, RpcCheckpointConfStatus, RpcCheckpointInfo, RpcCheckpointL1Ref,
    RpcOLBlockInfo, RpcOLChainStatus, RpcOLTransaction, RpcSnarkAccountState, RpcUpdateInputData,
};
use strata_primitives::{HexBytes, HexBytes32};
use strata_snark_acct_types::ProofState;
use tracing::{error, info};

use crate::rpc::errors::{
    db_error, internal_error, invalid_params_error, map_mempool_error_to_rpc, not_found_error,
};

/// OL RPC server implementation, generic over a provider.
pub(crate) struct OLRpcServer<P: OLRpcProvider> {
    provider: P,
    genesis_l1_height: L1Height,
    max_headers_range: usize,
}

impl<P: OLRpcProvider> OLRpcServer<P> {
    /// Creates a new [`OLRpcServer`].
    pub(crate) fn new(provider: P, genesis_l1_height: L1Height, max_headers_range: usize) -> Self {
        Self {
            provider,
            genesis_l1_height,
            max_headers_range,
        }
    }

    async fn get_canonical_block_at_height(&self, height: u64) -> RpcResult<Option<OLBlockId>> {
        let blkid = self
            .provider
            .get_canonical_block_at(height)
            .await
            .map_err(db_error)?
            .map(|b| b.blkid);
        Ok(blkid)
    }

    async fn get_block(&self, blkid: OLBlockId) -> RpcResult<OLBlock> {
        let blk = self
            .provider
            .get_block_data(blkid)
            .await
            .map_err(db_error)?
            .ok_or(not_found_error(format!("block not found: {blkid}")))?;
        Ok(blk)
    }

    async fn get_canonical_epoch_summary(
        &self,
        epoch: Epoch,
    ) -> RpcResult<Option<(EpochCommitment, EpochSummary)>> {
        let Some(commitment) = self
            .provider
            .get_canonical_epoch_commitment_at(epoch)
            .await
            .map_err(db_error)?
        else {
            return Ok(None);
        };

        let Some(summary) = self
            .provider
            .get_epoch_summary(commitment)
            .await
            .map_err(db_error)?
        else {
            return Ok(None);
        };

        Ok(Some((commitment, summary)))
    }

    async fn get_first_l2_block_in_epoch(
        &self,
        summary: &EpochSummary,
    ) -> RpcResult<L2BlockCommitment> {
        let prev_terminal_blkid = *summary.prev_terminal().blkid();
        let mut cur_blkid = *summary.terminal().blkid();
        // Parent links should move from terminal toward prev_terminal within this slot span.
        let max_hops = summary
            .terminal()
            .slot()
            .saturating_sub(summary.prev_terminal().slot())
            .saturating_add(1);
        let mut hops = 0u64;

        while hops <= max_hops {
            let block = self.get_block(cur_blkid).await?;
            let header = block.header();
            let parent = *header.parent_blkid();

            if parent == prev_terminal_blkid {
                return Ok(L2BlockCommitment::new(header.slot(), cur_blkid));
            }

            cur_blkid = parent;
            hops = hops.saturating_add(1);
        }

        Err(internal_error(format!(
            "Unable to derive first L2 block for epoch {} from terminal ancestry",
            summary.epoch()
        )))
    }

    async fn get_prev_epoch_commitment(&self, epoch: Epoch) -> RpcResult<EpochCommitment> {
        if epoch == 0 {
            return Ok(EpochCommitment::null());
        }

        self.provider
            .get_canonical_epoch_commitment_at(epoch - 1)
            .await
            .map_err(db_error)?
            .ok_or_else(|| {
                not_found_error(format!("No epoch commitment found for epoch {}", epoch - 1))
            })
    }

    async fn get_prev_terminal_next_inbox_msg_idx(
        &self,
        account_id: AccountId,
        prev_epoch_commitment: EpochCommitment,
    ) -> RpcResult<u64> {
        let prev_terminal_commitment = prev_epoch_commitment.to_block_commitment();
        let prev_ol_state = self
            .provider
            .get_toplevel_ol_state(prev_terminal_commitment)
            .await
            .map_err(|e| {
                error!(
                    ?e,
                    %prev_terminal_commitment,
                    "Failed to get previous epoch OL state"
                );
                db_error(e)
            })?
            .ok_or_else(|| {
                not_found_error(format!(
                    "No OL state found for previous terminal block {prev_terminal_commitment}"
                ))
            })?;

        let prev_next_inbox_msg_idx = prev_ol_state
            .get_account_state(&account_id)
            .and_then(|account_state| account_state.as_snark_account().ok())
            .map_or(0, |snark_state| snark_state.next_inbox_msg_idx());

        Ok(prev_next_inbox_msg_idx)
    }

    async fn fetch_account_epoch_messages(
        &self,
        account_id: AccountId,
        epoch: Epoch,
        current_next_inbox_idx: Option<u64>,
        prev_epoch_commitment: EpochCommitment,
    ) -> RpcResult<Vec<MessageEntry>> {
        // Epoch 0 is genesis-only in current flow: there is no prior terminal state and no
        // collectable inbox updates, so messages are always empty.
        if epoch == 0 {
            return Ok(Vec::new());
        }

        let Some(end_idx_exclusive) = current_next_inbox_idx else {
            return Ok(Vec::new());
        };
        if end_idx_exclusive == 0 {
            return Ok(Vec::new());
        }

        let start_idx = self
            .get_prev_terminal_next_inbox_msg_idx(account_id, prev_epoch_commitment)
            .await?;

        self.provider
            .get_account_inbox_messages(account_id, start_idx, end_idx_exclusive)
            .await
            .map_err(db_error)
    }
}

#[async_trait]
impl<P: OLRpcProvider> OLClientRpcServer for OLRpcServer<P> {
    async fn get_acct_epoch_summary(
        &self,
        account_id: AccountId,
        epoch: Epoch,
    ) -> RpcResult<RpcAccountEpochSummary> {
        // Get epoch commitments for the given epoch
        let epoch_commitment = self
            .provider
            .get_canonical_epoch_commitment_at(epoch)
            .await
            .map_err(|e| {
                error!(?e, ?epoch, "Failed to get canonical epoch commitment");
                db_error(e)
            })?
            .ok_or_else(|| {
                not_found_error(format!("No canonical commitment found for epoch {epoch}"))
            })?;

        // Get OL state at the terminal block using the epoch commitment directly
        // (EpochCommitment already contains the terminal slot and block ID)
        let terminal_commitment = epoch_commitment.to_block_commitment();
        let ol_state = self
            .provider
            .get_toplevel_ol_state(terminal_commitment)
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
            .get_account_state(&account_id)
            .ok_or_else(|| not_found_error(format!("Account {account_id} not found")))?;

        // Extract snark-specific data if applicable.
        // For non-snark accounts, these fields are zeroed since seqno and proof state
        // concepts don't apply to them.
        let (next_seq_no, proof_state, current_next_inbox_idx) =
            match account_state.as_snark_account() {
                Ok(snark_state) => {
                    let seqno: u64 = *snark_state.seqno().inner();
                    let inner_state = snark_state.inner_state_root();
                    let next_inbox_idx = snark_state.next_inbox_msg_idx();
                    (
                        seqno,
                        ProofState::new(inner_state, next_inbox_idx),
                        Some(next_inbox_idx),
                    )
                }
                Err(_) => (0, ProofState::new([0u8; 32].into(), 0), None), // Non-snark account
            };

        // Get previous epoch commitment if available
        let prev_epoch_commitment = self.get_prev_epoch_commitment(epoch).await?;

        let update = if let Some(extra_data) = self
            .provider
            .get_account_extra_data((account_id, epoch))
            .await
            .map_err(db_error)?
        {
            let messages = self
                .fetch_account_epoch_messages(
                    account_id,
                    epoch,
                    current_next_inbox_idx,
                    prev_epoch_commitment,
                )
                .await?;

            let update = RpcUpdateInputData {
                seq_no: next_seq_no,
                proof_state: proof_state.into(),
                extra_data: extra_data
                    .last() // FIXME: check if this is canonical or not and account for reorgs.
                    .cloned()
                    .expect("Should be present")
                    .into_parts()
                    .0
                    .into(),
                messages: messages.into_iter().map(Into::into).collect(),
            };
            Some(update)
        } else {
            None
        };

        Ok(RpcAccountEpochSummary::new(
            epoch_commitment,
            prev_epoch_commitment,
            account_state.balance().to_sat(),
            update,
        ))
    }

    async fn chain_status(&self) -> RpcResult<RpcOLChainStatus> {
        let chain_sync_status = self
            .provider
            .get_ol_sync_status()
            .ok_or_else(|| internal_error("OL sync status not available"))?;

        let tip = RpcOLBlockInfo::new(
            *chain_sync_status.tip.blkid(),
            chain_sync_status.tip.slot(),
            chain_sync_status.tip_epoch,
            chain_sync_status.tip_is_terminal,
        );
        let confirmed = chain_sync_status.confirmed_epoch;
        let finalized = chain_sync_status.finalized_epoch;

        Ok(RpcOLChainStatus::new(tip, confirmed, finalized))
    }

    async fn get_checkpoint_info(&self, epoch: Epoch) -> RpcResult<Option<RpcCheckpointInfo>> {
        let Some((commitment, epoch_summary)) = self.get_canonical_epoch_summary(epoch).await?
        else {
            return Ok(None);
        };
        let l2_start = self.get_first_l2_block_in_epoch(&epoch_summary).await?;
        let l2_range = (l2_start, *epoch_summary.terminal());

        let l1_start = if epoch == 0 {
            let l1_start_height = self.genesis_l1_height.saturating_add(1);
            let l1_start_manifest = self
                .provider
                .get_block_manifest_at_height(l1_start_height)
                .await
                .map_err(db_error)?
                .ok_or_else(|| {
                    not_found_error(format!(
                        "No L1 manifest found at genesis+1 height {} for epoch 0",
                        l1_start_height
                    ))
                })?;

            L1BlockCommitment::new(l1_start_height, *l1_start_manifest.blkid())
        } else {
            let prev_epoch = epoch - 1;
            let (_, prev_summary) = self
                .get_canonical_epoch_summary(prev_epoch)
                .await?
                .ok_or_else(|| {
                    not_found_error(format!("No canonical summary found for epoch {prev_epoch}"))
                })?;

            let l1_start_height = prev_summary.new_l1().height().saturating_add(1);
            let l1_start_manifest = self
                .provider
                .get_block_manifest_at_height(l1_start_height)
                .await
                .map_err(db_error)?
                .ok_or_else(|| {
                    not_found_error(format!(
                        "No L1 manifest found at checkpoint start height {} for epoch {}",
                        l1_start_height, epoch
                    ))
                })?;

            L1BlockCommitment::new(l1_start_height, *l1_start_manifest.blkid())
        };
        let l1_end = *epoch_summary.new_l1();
        let l1_range = (l1_start, l1_end);

        let l1_ref = self
            .provider
            .get_checkpoint_l1_ref(commitment)
            .await
            .map_err(db_error)?;
        let confirmation_status = if let Some(obs) = l1_ref {
            let l1_reference = RpcCheckpointL1Ref::new(obs.l1_commitment, obs.txid, obs.wtxid);
            let observed_height = obs.l1_commitment.height();
            let Some(tip) = self.provider.get_l1_tip_height() else {
                return Err(internal_error(
                    "L1 tip height unavailable while constructing checkpoint info",
                ));
            };
            if tip < observed_height {
                return Err(internal_error(format!(
                    "L1 tip height {tip} is below observed checkpoint height {observed_height}",
                )));
            }

            let is_finalized = self
                .provider
                .get_ol_sync_status()
                .is_some_and(|sync_status| sync_status.finalized_epoch.epoch() >= epoch);

            if is_finalized {
                RpcCheckpointConfStatus::Finalized { l1_reference }
            } else {
                RpcCheckpointConfStatus::Confirmed { l1_reference }
            }
        } else {
            RpcCheckpointConfStatus::Pending
        };

        Ok(Some(RpcCheckpointInfo {
            idx: epoch as u64,
            l1_range,
            l2_range,
            confirmation_status,
        }))
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
            .provider
            .get_ol_sync_status()
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
                .provider
                .get_toplevel_ol_state(block_commitment)
                .await
                .map_err(|e| {
                    error!(?e, %block_commitment, "Failed to get OL state");
                    db_error(e)
                })?;

            let Some(ol_state) = ol_state else {
                continue; // Skip if state not available
            };

            // Get account state
            let Some(account_state) = ol_state.get_account_state(&account_id) else {
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
                    let next_inbox_idx = snark_state.next_inbox_msg_idx();
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

    async fn get_account_genesis_epoch_commitment(
        &self,
        account_id: AccountId,
    ) -> RpcResult<EpochCommitment> {
        let epoch = self
            .provider
            .get_account_creation_epoch(account_id)
            .await
            .map_err(db_error)?
            .ok_or_else(|| {
                not_found_error(format!("No creation epoch found for account {account_id}"))
            })?;

        self.provider
            .get_canonical_epoch_commitment_at(epoch)
            .await
            .map_err(db_error)?
            .ok_or_else(|| not_found_error(format!("No epoch commitment found for epoch {epoch}")))
    }

    async fn get_l1_header_commitment(&self, l1_height: L1Height) -> RpcResult<Option<HexBytes32>> {
        let manifest = self
            .provider
            .get_block_manifest_at_height(l1_height)
            .await
            .map_err(db_error)?;

        Ok(manifest.map(|m| HexBytes32::from(m.compute_hash())))
    }

    async fn submit_transaction(&self, tx: RpcOLTransaction) -> RpcResult<OLTxId> {
        // Convert RPC transaction to mempool transaction
        let mempool_tx: OLTransaction = tx
            .try_into()
            .map_err(|e| invalid_params_error(format!("Invalid transaction: {e}")))?;
        let target = mempool_tx
            .target()
            .expect("all OL payload variants must have a target");
        let next_inbox_msg_idx = match mempool_tx.payload() {
            TransactionPayload::SnarkAccountUpdate(payload) => Some(
                payload
                    .operation()
                    .update()
                    .proof_state()
                    .new_next_msg_idx(),
            ),
            TransactionPayload::GenericAccountMessage(_) => None,
        };

        // Submit to mempool
        let txid = self
            .provider
            .submit_transaction(mempool_tx)
            .await
            .map_err(map_mempool_error_to_rpc)?;

        match next_inbox_msg_idx {
            Some(next_inbox_msg_idx) => {
                info!(
                    %txid,
                    %target,
                    next_inbox_msg_idx,
                    "snark update received by the OL mempool"
                );
            }
            None => {
                info!(
                    %txid,
                    %target,
                    "transaction received by the OL mempool"
                );
            }
        }

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
                    .provider
                    .get_ol_sync_status()
                    .ok_or_else(|| internal_error("OL sync status not available"))?;
                chain_sync_status.tip
            }
            OLBlockOrTag::Confirmed => {
                let chain_sync_status = self
                    .provider
                    .get_ol_sync_status()
                    .ok_or_else(|| internal_error("OL sync status not available"))?;
                // TODO: STR-2420 Address this incorrect use of prev_epoch as confirmed epoch
                chain_sync_status.prev_epoch.to_block_commitment()
            }
            OLBlockOrTag::Finalized => {
                let chain_sync_status = self
                    .provider
                    .get_ol_sync_status()
                    .ok_or_else(|| internal_error("OL sync status not available"))?;
                chain_sync_status.finalized_epoch.to_block_commitment()
            }
            OLBlockOrTag::OLBlockId(block_id) => {
                let block = self
                    .provider
                    .get_block_data(block_id)
                    .await
                    .map_err(|e| {
                        error!(?e, %block_id, "Failed to get block data");
                        db_error(e)
                    })?
                    .ok_or_else(|| not_found_error(format!("Block {block_id} not found")))?;
                OLBlockCommitment::new(block.header().slot(), block_id)
            }
            OLBlockOrTag::Slot(slot) => self
                .provider
                .get_canonical_block_at(slot)
                .await
                .map_err(db_error)?
                .ok_or_else(|| not_found_error(format!("No block found at slot {slot}")))?,
        };

        // Get OL state at the resolved block
        let ol_state = self
            .provider
            .get_toplevel_ol_state(block_commitment)
            .await
            .map_err(|e| {
                error!(?e, %block_commitment, "Failed to get OL state");
                db_error(e)
            })?
            .ok_or_else(|| {
                not_found_error(format!("No OL state found for block {block_commitment}"))
            })?;

        // Get account state
        let Some(account_state) = ol_state.get_account_state(&account_id) else {
            return Ok(None); // Account doesn't exist
        };

        // Try to get snark account state; return None if not a snark account
        match account_state.as_snark_account() {
            Ok(snark_state) => {
                // Note: update_vk is not available from NativeSnarkAccountState (it's stored
                // as account metadata, not runtime state), so we return an empty vec for now
                let seq_no: u64 = *snark_state.seqno().inner();
                let inner_state = snark_state.inner_state_root().0.into();
                let next_inbox_msg_idx = snark_state.next_inbox_msg_idx();
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
impl<P: OLRpcProvider> OLFullNodeRpcServer for OLRpcServer<P> {
    async fn get_raw_blocks_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> RpcResult<Vec<RpcBlockEntry>> {
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

    async fn get_headers_in_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> RpcResult<Vec<RpcBlockHeaderEntry>> {
        let block_count = (end_height.saturating_sub(start_height) + 1) as usize;

        if start_height > end_height || block_count > self.max_headers_range {
            return Err(invalid_params_error("Invalid block range"));
        }

        let last_blkid = self
            .get_canonical_block_at_height(end_height)
            .await?
            .ok_or(not_found_error(format!(
                "No blocks found at slot {end_height}"
            )))?;

        let mut cur_blkid = last_blkid;
        let mut entries = Vec::with_capacity(block_count);

        for _ in (start_height..=end_height).rev() {
            let blk = self.get_block(cur_blkid).await?;
            cur_blkid = blk.header().parent_blkid;
            entries.push(RpcBlockHeaderEntry::from(&blk));
        }
        entries.reverse();

        Ok(entries)
    }
}
