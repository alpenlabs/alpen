//! OL RPC server implementation for a strata node.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::Arc,
};

use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use ssz::{Decode, Encode};
use strata_acct_types::MessageEntry;
use strata_checkpoint_types::EpochSummary;
use strata_db_types::ol_state_index::InboxMessageRecord;
use strata_identifiers::{
    AccountId, Epoch, EpochCommitment, Hash, L1BlockCommitment, L1Height, L2BlockCommitment,
    OLBlockCommitment, OLBlockId, OLTxId,
};
use strata_ledger_types::{IAccountState, ISnarkAccountState};
use strata_ol_chain_types_new::{OLBlock, OLTransaction, TransactionPayload};
use strata_ol_rpc_api::{OLClientRpcServer, OLFullNodeRpcServer, OLSubmitRpcServer};
use strata_ol_rpc_types::{
    OLBlockOrTag, OLRpcProvider, RpcAccountBlockSummary, RpcAccountChange, RpcAccountChangeType,
    RpcAccountEpochSummary, RpcAccountState, RpcBlockAccountChanges, RpcBlockEntry,
    RpcBlockHeaderEntry, RpcCheckpointConfStatus, RpcCheckpointInfo, RpcCheckpointL1Ref,
    RpcIndexedEntry, RpcMessageEntry, RpcOLBlockDetail, RpcOLBlockInfo, RpcOLBlockSummary,
    RpcOLChainStatus, RpcOLTransaction, RpcOLTxDetail, RpcSnarkAccountState, RpcUpdateInputData,
};
use strata_ol_state_types::OLState;
use strata_primitives::{HexBytes, HexBytes32};
use strata_snark_acct_types::{ProofState, UpdateInputData, UpdateStateData};
use tracing::{error, info};

use crate::rpc::errors::{
    db_error, internal_error, invalid_params_error, map_mempool_error_to_rpc, not_found_error,
};

/// One canonical-chain block in the range walked by `get_blocks_summaries`.
struct ChainBlock {
    slot: u64,
    blkid: OLBlockId,
    epoch: Epoch,
}

/// OL RPC server implementation, generic over a provider.
pub(crate) struct OLRpcServer<P: OLRpcProvider> {
    provider: P,
    genesis_l1_height: L1Height,
    // Maximum number of headers/block-data that can be queried
    max_headers_range: usize,
}

/// Convenient wrapper for account records.
struct AccountRecords {
    updates_by_block: HashMap<OLBlockCommitment, Vec<UpdateInputData>>,
    inbox: Vec<InboxMessageRecord>,
}

/// Maximum number of Snark account inbox messages returned by one RPC call.
///
/// This is a server-side page-size and DoS guard, not a protocol limit. Callers
/// that need a larger inbox span should split it into multiple requests.
const MAX_SNARK_ACCT_INBOX_MSG_RANGE: u64 = 1_000;

fn local_inbox_message_range(
    account_id: AccountId,
    start_idx: u64,
    end_idx_exclusive: u64,
    fetched_start_idx: u64,
) -> RpcResult<Range<usize>> {
    let local_start = start_idx.checked_sub(fetched_start_idx).ok_or_else(|| {
        internal_error(format!(
            "account {account_id} inbox range [{start_idx}, {end_idx_exclusive}) \
                 starts before fetched range start {fetched_start_idx}"
        ))
    })?;
    let local_end = end_idx_exclusive
        .checked_sub(fetched_start_idx)
        .ok_or_else(|| {
            internal_error(format!(
                "account {account_id} inbox range [{start_idx}, {end_idx_exclusive}) \
                 ends before fetched range start {fetched_start_idx}"
            ))
        })?;
    if local_end < local_start {
        return Err(internal_error(format!(
            "account {account_id} inbox range has reversed bounds: \
             [{start_idx}, {end_idx_exclusive})"
        )));
    }

    let local_start = usize::try_from(local_start).map_err(|_| {
        internal_error(format!(
            "account {account_id} inbox range start offset {local_start} does not fit in usize"
        ))
    })?;
    let local_end = usize::try_from(local_end).map_err(|_| {
        internal_error(format!(
            "account {account_id} inbox range end offset {local_end} does not fit in usize"
        ))
    })?;

    Ok(local_start..local_end)
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

    /// Resolves an [`OLBlockOrTag`] to a concrete [`OLBlockCommitment`].
    ///
    /// `Latest`, `Confirmed`, and `Finalized` come from the OL sync status.
    /// `OLBlockId` requires fetching the block to read its slot. `Slot` looks
    /// up the canonical block at that slot.
    async fn resolve_block_or_tag(
        &self,
        block_or_tag: OLBlockOrTag,
    ) -> RpcResult<OLBlockCommitment> {
        Ok(match block_or_tag {
            OLBlockOrTag::Latest => {
                self.provider
                    .get_ol_sync_status()
                    .ok_or_else(|| internal_error("OL sync status not available"))?
                    .tip
            }
            OLBlockOrTag::Confirmed => self
                .provider
                .get_ol_sync_status()
                .ok_or_else(|| internal_error("OL sync status not available"))?
                .confirmed_epoch
                .to_block_commitment(),
            OLBlockOrTag::Finalized => self
                .provider
                .get_ol_sync_status()
                .ok_or_else(|| internal_error("OL sync status not available"))?
                .finalized_epoch
                .to_block_commitment(),
            OLBlockOrTag::OLBlockId(block_id) => {
                let block = self.get_block(block_id).await?;
                OLBlockCommitment::new(block.header().slot(), block_id)
            }
            OLBlockOrTag::Slot(slot) => self
                .provider
                .get_canonical_block_at(slot)
                .await
                .map_err(db_error)?
                .ok_or_else(|| not_found_error(format!("No block found at slot {slot}")))?,
        })
    }

    /// Walks the canonical chain backwards from `end_slot` to `start_slot`,
    /// returning blocks in ascending slot order. Each entry carries
    /// `(slot, blkid, epoch)`; epoch is read off the header during the walk.
    async fn collect_canonical_chain(
        &self,
        start_slot: u64,
        end_slot: u64,
    ) -> RpcResult<Vec<ChainBlock>> {
        let finalized_slot = self
            .provider
            .get_ol_sync_status()
            .map(|css| css.finalized_epoch.last_slot())
            .unwrap_or(0);

        let mut chain = Vec::new();

        let Some(end_block_id) = self.get_canonical_block_at_height(end_slot).await? else {
            return Ok(chain);
        };

        let mut current_id = end_block_id;
        loop {
            let block = self.get_block(current_id).await?;
            let header = block.header();
            let current_slot = header.slot();

            if current_slot >= start_slot && current_slot <= end_slot {
                chain.push(ChainBlock {
                    slot: current_slot,
                    blkid: current_id,
                    epoch: header.epoch(),
                });
            }

            if current_slot <= start_slot {
                break;
            }

            // Past the finalized boundary the chain is unique by slot, so we
            // can fetch remaining blocks directly without parent-walking.
            if current_slot <= finalized_slot {
                for slot in (start_slot..current_slot).rev() {
                    let Some(blkid) = self.get_canonical_block_at_height(slot).await? else {
                        continue;
                    };
                    let block = self.get_block(blkid).await?;
                    chain.push(ChainBlock {
                        slot,
                        blkid,
                        epoch: block.header().epoch(),
                    });
                }
                break;
            }

            current_id = *header.parent_blkid();
        }

        chain.reverse();
        Ok(chain)
    }

    /// Fetches per-(account, epoch) update and inbox records over
    /// `[first_epoch, last_epoch]`. Updates are grouped by block commitment,
    /// filtered to `block_commitments`. Out-of-chain and checkpoint-sync
    /// records still advance the inbox cursor so in-chain slices stay accurate.
    async fn fetch_records_in_epoch_range(
        &self,
        account_id: AccountId,
        first_epoch: Epoch,
        last_epoch: Epoch,
        block_commitments: &HashSet<OLBlockCommitment>,
    ) -> RpcResult<AccountRecords> {
        // Seed the cursor once. If the account isn't found in the prior epoch's
        // terminal state — either because `first_epoch == 0` (no prior epoch
        // to query) or because it didn't exist as a snark account at that
        // point — start at 0. Both cases mean "no prior record of the account
        // in queryable state, so its inbox starts at 0."
        let mut cursor: u64 = if first_epoch == 0 {
            0
        } else {
            let (_, prev_ol_state) = self
                .get_toplevel_ol_state_for_epoch(first_epoch - 1)
                .await?;
            prev_ol_state
                .get_account_state(&account_id)
                .and_then(|s| s.as_snark_account().ok())
                .map_or(0, |s| s.next_inbox_msg_idx())
        };

        // Walk records across all epochs in one pass. Out-of-chain and
        // checkpoint-sync rows advance the cursor without emitting; in-chain
        // records emit a `Pending` triple capturing the inbox slice they
        // consumed. Inbox indexing is monotonic per account across epoch
        // boundaries, so the cursor flows through without re-seeding.
        struct Pending {
            block_commitment: OLBlockCommitment,
            seq_no: u64,
            new_state_root: Hash,
            extra_data: Vec<u8>,
            cursor_start: u64,
            cursor_end: u64,
        }

        let mut pending: Vec<Pending> = Vec::new();
        let mut all_inbox = Vec::new();
        for epoch in first_epoch..=last_epoch {
            if let Some(records) = self
                .provider
                .get_account_update_records(epoch, account_id)
                .await
                .map_err(db_error)?
            {
                for r in records {
                    let prev_cursor = cursor;
                    let next_cursor = r.next_inbox_idx();
                    cursor = next_cursor;

                    // Skip rows with no block attribution: checkpoint-sync,
                    // or CSS-terminal-stamped rows (root present but block
                    // absent). This endpoint is the block-scoped view.
                    let Some(meta) = r.update_meta() else {
                        continue;
                    };
                    let Some(block_commitment) = meta.block_commitment().copied() else {
                        continue;
                    };

                    // Out-of-chain blocks: belong to a sibling/orphan that's
                    // not on the queried canonical chain.
                    if !block_commitments.contains(&block_commitment) {
                        continue;
                    }

                    // Block-attributed `DirectSet` (no `extra_data`) is not
                    // produced by current write paths. Keep the soft fail in
                    // case the invariant ever breaks.
                    let extra_data = r
                        .extra_data()
                        .ok_or_else(|| {
                            internal_error(format!(
                                "update record for account {account_id} epoch {epoch} \
                                 has no extra_data (DirectSet)"
                            ))
                        })?
                        .to_vec();

                    pending.push(Pending {
                        block_commitment,
                        seq_no: r.seq_no(),
                        new_state_root: meta.new_state_root(),
                        extra_data,
                        cursor_start: prev_cursor,
                        cursor_end: next_cursor,
                    });
                }
            }
            if let Some(records) = self
                .provider
                .get_account_inbox_records(epoch, account_id)
                .await
                .map_err(db_error)?
            {
                all_inbox.extend(records);
            }
        }

        // Single inbox fetch covering every emitted record's `[start, end)`.
        // Slice locally per record. Skip the fetch entirely at genesis or
        // when no records survived — message slicing is undefined there.
        let mut updates_by_block: HashMap<OLBlockCommitment, Vec<UpdateInputData>> = HashMap::new();
        if !pending.is_empty() && first_epoch != 0 {
            let min_start = pending.iter().map(|p| p.cursor_start).min().unwrap_or(0);
            let max_end = pending.iter().map(|p| p.cursor_end).max().unwrap_or(0);
            let messages_in_range = if max_end > min_start {
                self.provider
                    .get_account_inbox_messages(account_id, min_start, max_end)
                    .await
                    .map_err(db_error)?
            } else {
                Vec::new()
            };

            for p in pending {
                let message_range =
                    local_inbox_message_range(account_id, p.cursor_start, p.cursor_end, min_start)?;
                let messages = messages_in_range[message_range].to_vec();
                updates_by_block
                    .entry(p.block_commitment)
                    .or_default()
                    .push(UpdateInputData::new(
                        p.seq_no,
                        messages,
                        UpdateStateData::new(
                            ProofState::new(p.new_state_root, p.cursor_end),
                            p.extra_data,
                        ),
                    ));
            }
        } else {
            // Genesis epoch: no messages can have been consumed pre-genesis,
            // so each record gets an empty `processed_messages` vec.
            for p in pending {
                updates_by_block
                    .entry(p.block_commitment)
                    .or_default()
                    .push(UpdateInputData::new(
                        p.seq_no,
                        Vec::new(),
                        UpdateStateData::new(
                            ProofState::new(p.new_state_root, p.cursor_end),
                            p.extra_data,
                        ),
                    ));
            }
        }

        Ok(AccountRecords {
            updates_by_block,
            inbox: all_inbox,
        })
    }

    /// Builds one block summary from records already filtered to this block.
    /// Returns `Ok(None)` when state or account is unavailable at this block.
    async fn build_block_summary(
        &self,
        account_id: AccountId,
        cb: &ChainBlock,
        block_updates: Vec<UpdateInputData>,
        block_inbox: &[&InboxMessageRecord],
    ) -> RpcResult<Option<RpcAccountBlockSummary>> {
        let block_commitment = OLBlockCommitment::new(cb.slot, cb.blkid);

        let ol_state = self
            .provider
            .get_toplevel_ol_state(block_commitment)
            .await
            .map_err(|e| {
                error!(?e, %block_commitment, "Failed to get OL state");
                db_error(e)
            })?;
        let Some(ol_state) = ol_state else {
            return Ok(None);
        };

        let Some(account_state) = ol_state.get_account_state(&account_id) else {
            return Ok(None);
        };

        // Snark-only fields are zeroed for non-snark accounts. For block
        // summaries, `next_inbox_msg_idx` tracks the inbox accumulator leaf
        // count after this block, not the snark proof-state read cursor.
        let (next_seq_no, next_inbox_msg_idx) = match account_state.as_snark_account() {
            Ok(snark_state) => (
                *snark_state.seqno().inner(),
                snark_state.inbox_mmr().num_entries(),
            ),
            Err(_) => (0, 0),
        };

        let new_inbox_messages: Vec<MessageEntry> = block_inbox
            .iter()
            .map(|r| {
                MessageEntry::from_ssz_bytes(r.entry_bytes()).map_err(|e| {
                    internal_error(format!(
                        "failed to decode inbox record bytes for account {account_id} \
                         block {block_commitment}: {e}"
                    ))
                })
            })
            .collect::<RpcResult<Vec<_>>>()?;

        Ok(Some(RpcAccountBlockSummary::new(
            account_id,
            block_commitment,
            account_state.balance(),
            next_seq_no,
            block_updates,
            new_inbox_messages,
            next_inbox_msg_idx,
        )))
    }

    /// Resolves an epoch to its terminal-block OL state. Errors if either the
    /// canonical commitment or the terminal-block state is missing.
    async fn get_toplevel_ol_state_for_epoch(
        &self,
        epoch: Epoch,
    ) -> RpcResult<(EpochCommitment, Arc<OLState>)> {
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

        Ok((epoch_commitment, ol_state))
    }
}

#[async_trait]
impl<P: OLRpcProvider> OLClientRpcServer for OLRpcServer<P> {
    async fn get_acct_epoch_summary(
        &self,
        account_id: AccountId,
        epoch: Epoch,
    ) -> RpcResult<RpcAccountEpochSummary> {
        let (epoch_commitment, ol_state) = self.get_toplevel_ol_state_for_epoch(epoch).await?;
        let account_state = ol_state
            .get_account_state(&account_id)
            .ok_or_else(|| not_found_error(format!("Account {account_id} not found")))?;

        let snark_state = account_state.as_snark_account().map_err(|_| {
            invalid_params_error(format!("Account {account_id} is not a snark account"))
        })?;
        let final_state_root: HexBytes32 = snark_state.inner_state_root().0.into();

        let prev_epoch_commitment = self.get_prev_epoch_commitment(epoch).await?;

        let updates = if let Some(records) = self
            .provider
            .get_account_update_records(epoch, account_id)
            .await
            .map_err(db_error)?
        {
            if records.is_empty() {
                return Err(internal_error(format!(
                    "indexing entry for account {account_id} epoch {epoch} has no records"
                )));
            }

            // Inbox-message ranges are contiguous per record:
            // [prev_record.next_inbox_idx, this.next_inbox_idx). The first
            // record's lower bound is the prior epoch's terminal next_inbox_idx.
            // Epoch 0 is genesis: no prior terminal state, no collectable messages.
            let skip_fetch = epoch == 0;
            let mut cursor = if skip_fetch {
                0
            } else {
                let (_, prev_ol_state) = self.get_toplevel_ol_state_for_epoch(epoch - 1).await?;
                prev_ol_state
                    .get_account_state(&account_id)
                    .and_then(|s| s.as_snark_account().ok())
                    .map_or(0, |s| s.next_inbox_msg_idx())
            };

            struct Pending {
                seq_no: u64,
                new_state_root: Option<Hash>,
                extra_data: Vec<u8>,
                cursor_start: u64,
                cursor_end: u64,
            }

            let mut pending = Vec::with_capacity(records.len());
            for r in &records {
                let cursor_start = cursor;
                let cursor_end = r.next_inbox_idx();
                cursor = cursor_end;

                let new_state_root = r.update_meta().map(|m| m.new_state_root());
                let extra_data = r
                    .extra_data()
                    .ok_or_else(|| {
                        internal_error(format!(
                            "update record for account {account_id} epoch {epoch} \
                             has no extra_data (DirectSet)"
                        ))
                    })?
                    .to_vec();

                pending.push(Pending {
                    seq_no: r.seq_no(),
                    new_state_root,
                    extra_data,
                    cursor_start,
                    cursor_end,
                });
            }

            let min_start = pending.iter().map(|p| p.cursor_start).min().unwrap_or(0);
            let max_end = pending.iter().map(|p| p.cursor_end).max().unwrap_or(0);
            let messages_in_range = if skip_fetch || max_end <= min_start {
                Vec::new()
            } else {
                self.provider
                    .get_account_inbox_messages(account_id, min_start, max_end)
                    .await
                    .map_err(db_error)?
            };

            let mut out = Vec::with_capacity(pending.len());
            for p in pending {
                let messages = if skip_fetch {
                    Vec::new()
                } else {
                    let message_range = local_inbox_message_range(
                        account_id,
                        p.cursor_start,
                        p.cursor_end,
                        min_start,
                    )?;
                    messages_in_range[message_range].to_vec()
                };
                out.push(RpcUpdateInputData {
                    seq_no: p.seq_no,
                    next_inbox_msg_idx: p.cursor_end,
                    new_state_root: p.new_state_root.map(|root| root.0.into()),
                    extra_data: p.extra_data.into(),
                    messages: messages.into_iter().map(Into::into).collect(),
                });
            }
            out
        } else {
            Vec::new()
        };

        Ok(RpcAccountEpochSummary::new(
            epoch_commitment,
            prev_epoch_commitment,
            account_state.balance().to_sat(),
            final_state_root,
            updates,
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
        let latest = chain_sync_status.prev_epoch;

        Ok(RpcOLChainStatus::new(tip, confirmed, finalized, latest))
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
            let Some(tip) = self.provider.get_l1_tip_height().await.map_err(db_error)? else {
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
        let requested_block_count = end_slot.saturating_sub(start_slot).saturating_add(1);
        if requested_block_count as usize > self.max_headers_range {
            return Err(invalid_params_error(format!(
                "Block range too big. Allowed range is {}",
                self.max_headers_range
            )));
        }

        let chain_blocks = self.collect_canonical_chain(start_slot, end_slot).await?;
        if chain_blocks.is_empty() {
            return Ok(Vec::new());
        }

        // Pre-fetch indexing records across the chain's epoch span. Epochs
        // along the canonical chain are monotonic, so the touched set is a
        // contiguous range.
        let first_epoch = chain_blocks
            .first()
            .expect("non-empty chain blocks expected")
            .epoch;
        let last_epoch = chain_blocks
            .last()
            .expect("non-empty chain blocks expected")
            .epoch;
        let block_commitments: HashSet<OLBlockCommitment> = chain_blocks
            .iter()
            .map(|cb| OLBlockCommitment::new(cb.slot, cb.blkid))
            .collect();
        let AccountRecords {
            mut updates_by_block,
            inbox: all_inbox,
        } = self
            .fetch_records_in_epoch_range(account_id, first_epoch, last_epoch, &block_commitments)
            .await?;

        // Index inbox records by block_commitment so each block lookup is O(1)
        // instead of an O(M) scan. Inbox writes with no block tag can never
        // match a chain block, so they're dropped here.
        let mut inbox_by_block: HashMap<OLBlockCommitment, Vec<&InboxMessageRecord>> =
            HashMap::new();
        for r in &all_inbox {
            if let Some(c) = r.block_commitment() {
                inbox_by_block.entry(*c).or_default().push(r);
            }
        }

        let mut summaries = Vec::with_capacity(chain_blocks.len());
        for cb in &chain_blocks {
            let commitment = OLBlockCommitment::new(cb.slot, cb.blkid);
            let updates = updates_by_block.remove(&commitment).unwrap_or_default();
            let inbox = inbox_by_block
                .get(&commitment)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            if let Some(summary) = self
                .build_block_summary(account_id, cb, updates, inbox)
                .await?
            {
                summaries.push(summary);
            }
        }

        Ok(summaries)
    }

    async fn get_snark_acct_inbox_msg_range(
        &self,
        account_id: AccountId,
        start: u64,
        end: u64,
    ) -> RpcResult<Vec<RpcIndexedEntry<RpcMessageEntry>>> {
        if start > end {
            return Err(invalid_params_error("start must be <= end"));
        }
        let requested_message_count = end - start;
        if requested_message_count > MAX_SNARK_ACCT_INBOX_MSG_RANGE {
            return Err(invalid_params_error(format!(
                "Inbox message range too big \
                 (count {requested_message_count}, max {MAX_SNARK_ACCT_INBOX_MSG_RANGE})",
            )));
        }

        // NOTE: This intentionally does not pre-validate account existence or
        // account type. The provider/MMR path owns that behavior: empty ranges
        // return empty, while non-empty missing inbox data surfaces as a
        // storage error.
        let messages = self
            .provider
            .get_account_inbox_messages(account_id, start, end)
            .await
            .map_err(db_error)?;
        debug_assert!(
            u64::try_from(messages.len()).expect("message count must fit in u64") <= end - start,
            "provider returned more inbox messages than requested range"
        );

        Ok(messages
            .into_iter()
            .enumerate()
            .map(|(offset, message)| RpcIndexedEntry::new(start + offset as u64, message.into()))
            .collect())
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

    async fn get_asm_manifest_commitment(
        &self,
        l1_height: L1Height,
    ) -> RpcResult<Option<HexBytes32>> {
        let manifest = self
            .provider
            .get_block_manifest_at_height(l1_height)
            .await
            .map_err(db_error)?;

        Ok(manifest.map(|m| HexBytes32::from(*m.compute_hash().as_ref())))
    }

    async fn get_snark_account_state(
        &self,
        account_id: AccountId,
        block_or_tag: OLBlockOrTag,
    ) -> RpcResult<Option<RpcSnarkAccountState>> {
        let block_commitment = self.resolve_block_or_tag(block_or_tag).await?;

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
                let seq_no: u64 = *snark_state.seqno().inner();
                let inner_state = snark_state.inner_state_root().0.into();
                let next_inbox_msg_idx = snark_state.next_inbox_msg_idx();
                let update_vk = snark_state.update_vk().clone();

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

const MAX_RAW_BLOCKS_RANGE: usize = 5000;

#[async_trait]
impl<P: OLRpcProvider> OLSubmitRpcServer for OLRpcServer<P> {
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
}

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

    async fn get_block_by_slot(&self, slot: u64) -> RpcResult<Option<RpcOLBlockDetail>> {
        let Some(blkid) = self.get_canonical_block_at_height(slot).await? else {
            return Ok(None);
        };
        let block = self.get_block(blkid).await?;
        Ok(Some(RpcOLBlockDetail::from(&block)))
    }

    async fn get_recent_blocks(&self, count: u64) -> RpcResult<Vec<RpcOLBlockSummary>> {
        if count == 0 {
            return Ok(Vec::new());
        }
        if count as usize > self.max_headers_range {
            return Err(invalid_params_error(format!(
                "count {} exceeds max_headers_range {}",
                count, self.max_headers_range
            )));
        }

        // Walk parents from the sync-status tip directly so we read a consistent
        // chain anchored to the tip we observed (rather than re-resolving the
        // canonical block at the tip slot, which costs an extra DB hit and could
        // disagree with the snapshot if a reorg races us).
        let mut cur_blkid = *self
            .provider
            .get_ol_sync_status()
            .ok_or_else(|| internal_error("OL sync status not available"))?
            .tip
            .blkid();

        let mut summaries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let block = self.get_block(cur_blkid).await?;
            let header = block.header();
            summaries.push(RpcOLBlockSummary::from(&block));
            if header.slot() == 0 {
                break;
            }
            cur_blkid = *header.parent_blkid();
        }
        summaries.reverse();
        Ok(summaries)
    }

    async fn get_block_transactions(&self, slot: u64) -> RpcResult<Vec<RpcOLTxDetail>> {
        let blkid = self
            .get_canonical_block_at_height(slot)
            .await?
            .ok_or_else(|| not_found_error(format!("No block found at slot {slot}")))?;
        let block = self.get_block(blkid).await?;
        let txs = block
            .body()
            .tx_segment()
            .map(|seg| seg.txs().iter().map(RpcOLTxDetail::from).collect())
            .unwrap_or_default();
        Ok(txs)
    }

    async fn get_block_account_changes(&self, slot: u64) -> RpcResult<RpcBlockAccountChanges> {
        let block_commitment = self
            .provider
            .get_canonical_block_at(slot)
            .await
            .map_err(db_error)?
            .ok_or_else(|| not_found_error(format!("No block found at slot {slot}")))?;
        let write_batch = self
            .provider
            .get_ol_write_batch(block_commitment)
            .await
            .map_err(db_error)?
            .ok_or_else(|| {
                not_found_error(format!(
                    "No OL write batch found for block {block_commitment}"
                ))
            })?;
        let created_accounts: HashSet<AccountId> = write_batch
            .ledger()
            .new_accounts()
            .iter()
            .copied()
            .collect();
        let changes = write_batch
            .ledger()
            .iter_accounts()
            .map(|(id, state)| {
                let change_type = if created_accounts.contains(id) {
                    RpcAccountChangeType::Created
                } else {
                    RpcAccountChangeType::Updated
                };
                RpcAccountChange::new(*id, change_type, RpcAccountState::from(state))
            })
            .collect();

        Ok(RpcBlockAccountChanges::new(
            slot,
            block_commitment.blkid,
            changes,
        ))
    }
}
