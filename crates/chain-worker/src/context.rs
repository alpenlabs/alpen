//! Concrete implementation of the [`ChainWorkerContext`] trait.
//!
//! This module provides [`ChainWorkerContextImpl`], a production implementation
//! of the worker context that uses the storage layer managers for database access.

use std::{collections::BTreeMap, sync::Arc};

use ssz::Encode;
use strata_acct_types::{
    MessageEntry,
    tree_hash::{Sha256Hasher, TreeHash},
};
use strata_asm_common::AsmManifest;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_bridge_params::BridgeParams;
use strata_checkpoint_types::EpochSummary;
use strata_db_types::{
    errors::DbError,
    ol_state_index::{AccountUpdateMeta, AccountUpdateRecord, InboxMessageRecord, IndexingWrites},
};
use strata_identifiers::{AccountId, Hash, OLBlockCommitment, OLBlockId};
use strata_msg_fmt::{Msg, MsgRef};
use strata_node_context::NodeContext;
use strata_ol_chain_types::{
    OLBlock, OLBlockHeader, OLLog, OLLogType, SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID,
    SnarkAccountUpdateLogData,
};
use strata_ol_params::OLParams;
use strata_ol_state_types::{MMR_SENTINEL_DUMMY_LEAF_HASH, OLAccountState, OLState, WriteBatch};
use strata_primitives::epoch::EpochCommitment;
use strata_status::StatusChannel;
use strata_storage::{
    L1BlockManager, MmrId, MmrIndexManager, OLBlockManager, OLCheckpointManager,
    OLStateIndexingManager, OLStateManager,
};
use tokio::{runtime::Handle, sync::watch};
use tracing::{debug, error};

use crate::{
    errors::{WorkerError, WorkerResult},
    output::OLBlockExecutionOutput,
    traits::ChainWorkerContext,
};

/// Concrete implementation of [`ChainWorkerContext`] using storage managers.
///
/// This implementation wraps the high-level storage managers to provide
/// database access for the chain worker. All operations are blocking as
/// the worker runs on a dedicated thread pool.
#[expect(
    missing_debug_implementations,
    reason = "Storage managers don't implement Debug"
)]
pub struct ChainWorkerContextImpl {
    /// Manager for OL block data (headers + bodies).
    ol_block_mgr: Arc<OLBlockManager>,

    /// Manager for OL state snapshots and write batches.
    ol_state_mgr: Arc<OLStateManager>,

    /// Manager for checkpoint and epoch summary data.
    ol_checkpoint_mgr: Arc<OLCheckpointManager>,

    /// Manager for OL state indexing data (per-block writes, epoch finalization).
    ol_state_indexing_mgr: Arc<OLStateIndexingManager>,

    /// Manager for L1 block data, used to read ASM manifests by height.
    l1_block_mgr: Arc<L1BlockManager>,

    /// Manager for append-only MMR proof indices.
    mmr_index_mgr: Arc<MmrIndexManager>,

    /// Status channel to send/receive messages.
    status_channel: Arc<StatusChannel>,

    /// Channel for emitting epoch summary events.
    epoch_summary_tx: watch::Sender<Option<EpochCommitment>>,

    /// OL genesis params. Source of truth for the genesis L1 height used to
    /// prefill the L1 block refs MMR mirror (matches the in-state MMR which is
    /// seeded from `OLParams.last_l1_block.height()` at OL genesis).
    ol_params: Arc<OLParams>,

    /// Withdrawal denomination and cap.
    bridge_params: BridgeParams,

    /// Runtime handle
    handle: Handle,
}

impl ChainWorkerContextImpl {
    /// Creates a new context with the given storage managers.
    pub fn from_node_context(nodectx: &NodeContext) -> Self {
        let (epoch_summary_tx, _) = watch::channel(None);
        Self {
            ol_block_mgr: nodectx.storage().ol_block().clone(),
            ol_state_mgr: nodectx.storage().ol_state().clone(),
            ol_checkpoint_mgr: nodectx.storage().ol_checkpoint().clone(),
            ol_state_indexing_mgr: nodectx.storage().ol_state_indexing().clone(),
            l1_block_mgr: nodectx.storage().l1().clone(),
            mmr_index_mgr: nodectx.storage().mmr_index().clone(),
            status_channel: nodectx.status_channel().clone(),
            epoch_summary_tx,
            ol_params: nodectx.ol_params().clone(),
            bridge_params: *nodectx.ol_params().bridge_params(),
            handle: nodectx.executor().handle().clone(),
        }
    }

    pub fn epoch_summary_sender(&self) -> watch::Sender<Option<EpochCommitment>> {
        self.epoch_summary_tx.clone()
    }

    pub fn status_channel(&self) -> &StatusChannel {
        &self.status_channel
    }

    pub fn bridge_params(&self) -> BridgeParams {
        self.bridge_params
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }
}

impl ChainWorkerContext for ChainWorkerContextImpl {
    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlock>> {
        Ok(self.ol_block_mgr.get_block_data_blocking(*blkid)?)
    }

    fn fetch_blocks_at_slot(&self, slot: u64) -> WorkerResult<Vec<OLBlockId>> {
        Ok(self.ol_block_mgr.get_blocks_at_height_blocking(slot)?)
    }

    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeader>> {
        // Fetch the full block and extract just the header
        let block_opt = self.ol_block_mgr.get_block_data_blocking(*blkid)?;
        Ok(block_opt.map(|block| block.header().clone()))
    }

    fn fetch_chain_tip(&self) -> WorkerResult<Option<OLBlockCommitment>> {
        match self.ol_block_mgr.get_canonical_tip_blocking() {
            Ok(tip) => Ok(tip),
            Err(DbError::NotBootstrapped) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLState>> {
        let state_opt = self
            .ol_state_mgr
            .get_toplevel_ol_state_blocking(commitment)?;
        Ok(state_opt.map(|arc| (*arc).clone()))
    }

    fn fetch_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<WriteBatch<OLAccountState>>> {
        Ok(self.ol_state_mgr.get_write_batch_blocking(commitment)?)
    }

    /// Stores write batchees as well as indexing data.
    fn store_block_output(
        &self,
        block: &OLBlock,
        commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        let epoch = block.header().epoch();
        let wb = output.write_batch();

        self.ol_state_mgr
            .put_write_batch_blocking(commitment, wb.clone())?;

        let writes = build_indexing_writes(commitment, output)?;
        match self
            .ol_state_indexing_mgr
            .apply_block_indexing_blocking(epoch, commitment, writes)
        {
            Ok(()) => {
                index_mmr_writes(&self.mmr_index_mgr, output)?;
            }
            Err(DbError::BlockIndexingConflict {
                attempted,
                last_applied,
                ..
            }) if attempted == commitment && last_applied == commitment => {
                index_mmr_writes(&self.mmr_index_mgr, output)?;
                debug!(%commitment, "block indexing already applied; treating as retry");
            }
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }

    fn store_toplevel_state(
        &self,
        commitment: OLBlockCommitment,
        state: OLState,
    ) -> WorkerResult<()> {
        self.ol_state_mgr
            .put_toplevel_ol_state_blocking(commitment, state)?;
        Ok(())
    }

    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()> {
        let commitment = summary.get_epoch_commitment();

        // Idempotent: Stamp the commitment onto the indexing row first.
        self.ol_state_indexing_mgr
            .set_epoch_commitment_blocking(commitment.epoch(), commitment)?;

        // Insert the epoch summary last which indicates that the whole finalization persisted
        match self
            .ol_checkpoint_mgr
            .insert_epoch_summary_blocking(summary)
        {
            Ok(()) => {}
            Err(DbError::OverwriteEpoch(c)) if c == commitment => {
                let existing = self
                    .ol_checkpoint_mgr
                    .get_epoch_summary_blocking(commitment)?
                    .ok_or_else(|| {
                        WorkerError::Unexpected(format!(
                            "OverwriteEpoch reported but get_epoch_summary returned None for {commitment}"
                        ))
                    })?;
                if existing != summary {
                    return Err(WorkerError::Database(DbError::OverwriteEpoch(commitment)));
                }
                debug!(
                    %commitment,
                    "epoch summary already inserted with matching contents; \
                     treating as crash-restart retry"
                );
            }
            Err(e) => return Err(e.into()),
        }

        let _ = self.epoch_summary_tx.send(Some(commitment));
        Ok(())
    }

    fn fetch_canonical_epoch_summary_at(&self, epoch: u32) -> WorkerResult<Option<EpochSummary>> {
        let commitment = self
            .ol_checkpoint_mgr
            .get_canonical_epoch_commitment_at_blocking(epoch)?;
        if let Some(com) = commitment {
            Ok(self.ol_checkpoint_mgr.get_epoch_summary_blocking(com)?)
        } else {
            Ok(None)
        }
    }

    fn merge_epoch_data(&self, summary: &EpochSummary) -> WorkerResult<()> {
        let terminal = *summary.terminal();
        let prev_terminal = *summary.prev_terminal();

        // Collect canonical chain by walking backwards from terminal via parent pointers.
        // This ensures we only apply write batches for blocks in the canonical chain,
        // not fork blocks that may also have write batches stored.
        let mut chain: Vec<OLBlockCommitment> = Vec::new();
        let mut current = terminal;

        while current != prev_terminal && !current.is_null() {
            chain.push(current);
            // Get header to find parent
            let header = self
                .fetch_header(current.blkid())?
                .ok_or(WorkerError::MissingOLBlock(*current.blkid()))?;
            let parent_blkid = header.parent_blkid();
            if parent_blkid.is_null() {
                break;
            }
            current = OLBlockCommitment::new(current.slot().saturating_sub(1), *parent_blkid);
        }

        // Reverse to get forward order (excluding prev_terminal which is already finalized)
        chain.reverse();

        // Fetch prev state.
        let mut cur_state = self
            .fetch_ol_state(prev_terminal)?
            .ok_or(WorkerError::MissingPreState(prev_terminal))?;

        // Apply write batches in canonical order.
        // Every block in the canonical chain must have a write batch - a missing one
        // indicates data corruption or a bug, so we error out rather than skip.
        for commitment in chain {
            let wb = self
                .fetch_write_batch(commitment)?
                .ok_or(WorkerError::MissingWriteBatch(commitment))?;
            cur_state
                .apply_write_batch(wb)
                .map_err(|e| WorkerError::Unexpected(format!("failed to apply batch: {e}")))?;
        }

        // Store the final merged state at the terminal commitment
        self.ol_state_mgr
            .put_toplevel_ol_state_blocking(terminal, cur_state)?;

        Ok(())
    }

    fn prefill_l1_block_refs_mmr(&self) -> WorkerResult<()> {
        // Same source as the in-state MMR's genesis prefill so the two MMRs
        // stay byte-identical from leaf 0.
        let genesis_l1_height = self.ol_params.last_l1_block.height() as u64;
        prefill_l1_block_refs_mmr_blocking(&self.mmr_index_mgr, genesis_l1_height)
    }

    fn fetch_checkpoint_payload(
        &self,
        epoch: &EpochCommitment,
    ) -> WorkerResult<Option<CheckpointPayload>> {
        Ok(self
            .ol_checkpoint_mgr
            .get_checkpoint_l1_observed_payload_blocking(*epoch)?)
    }

    fn fetch_l1_manifests(&self, from: u32, to: u32) -> WorkerResult<Vec<AsmManifest>> {
        let mut manifests = Vec::new();
        for height in from..=to {
            let manifest = self
                .l1_block_mgr
                .get_block_manifest_at_height(height)?
                .ok_or(WorkerError::MissingDependency("l1 manifest"))?;
            manifests.push(manifest);
        }
        Ok(manifests)
    }

    fn apply_epoch_indexing(
        &self,
        epoch: &EpochCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        let writes = build_checkpoint_indexing_writes(output)?;
        self.ol_state_indexing_mgr
            .apply_epoch_indexing_blocking(*epoch, writes)?;
        index_mmr_writes(&self.mmr_index_mgr, output)?;
        Ok(())
    }
}

/// Builds an [`IndexingWrites`] payload from a block-execution output.
///
/// Reads account-creation events, snark-account update records (each tagged with the block's
/// commitment + final state root), and inbox-message writes (encoded as SSZ bytes) from the
/// block's [`IndexerWrites`]. Per-account vecs preserve insertion order.
///
/// Each snark-account update's `extra_data` is sourced from the emitted
/// [`SnarkAccountUpdateLogData`] logs rather than from the state-accessor layer. Every tracked
/// snark state update corresponds to exactly one such log (both are produced together when an
/// update transaction is processed), so the ordered updates are paired 1:1 with the ordered
/// snark-update logs.
fn build_indexing_writes(
    commitment: OLBlockCommitment,
    output: &OLBlockExecutionOutput,
) -> WorkerResult<IndexingWrites> {
    let indexer_writes = output.indexer_writes();

    let created_accounts: Vec<AccountId> = indexer_writes
        .created_accounts()
        .iter()
        .map(|c| c.account_id())
        .collect();

    let snark_updates = indexer_writes.snark_state_updates();
    let snark_update_logs = collect_snark_update_logs(output.logs())?;
    if snark_updates.len() != snark_update_logs.len() {
        return Err(WorkerError::SnarkUpdateLogCountMismatch {
            expected: snark_updates.len(),
            found: snark_update_logs.len(),
        });
    }

    let mut account_updates: BTreeMap<AccountId, Vec<AccountUpdateRecord>> = BTreeMap::new();
    for (update, log) in snark_updates.iter().zip(snark_update_logs.iter()) {
        // The paired log must describe the same state transition as the tracked update.
        if log.new_msg_idx() != update.next_read_idx() {
            return Err(WorkerError::SnarkUpdateLogMismatch {
                expected: update.next_read_idx(),
                found: log.new_msg_idx(),
            });
        }
        let state = update
            .state()
            .expect("block-sync snark update must carry a state root");
        let meta = AccountUpdateMeta::new(Some(commitment), state);
        let record = AccountUpdateRecord::new(
            Some(meta),
            *update.seqno().inner(),
            update.prev_next_read_idx(),
            update.next_read_idx(),
            Some(log.extra_data().to_vec()),
        );
        account_updates
            .entry(update.account_id())
            .or_default()
            .push(record);
    }

    debug_assert_contiguous_update_ranges(&account_updates);

    let mut account_inbox_writes: BTreeMap<AccountId, Vec<InboxMessageRecord>> = BTreeMap::new();
    for write in indexer_writes.inbox_messages() {
        let entry_bytes = write.entry().as_ssz_bytes();
        let record = InboxMessageRecord::new(entry_bytes, Some(commitment));
        account_inbox_writes
            .entry(write.account_id())
            .or_default()
            .push(record);
    }

    Ok(IndexingWrites::new(
        created_accounts,
        account_updates,
        account_inbox_writes,
    ))
}

/// Decodes the [`SnarkAccountUpdateLogData`] logs from `logs`, in emission order, skipping logs
/// of other types.
fn collect_snark_update_logs<'a>(
    logs: impl IntoIterator<Item = &'a OLLog>,
) -> WorkerResult<Vec<SnarkAccountUpdateLogData>> {
    let mut out = Vec::new();
    for log in logs {
        let msg = MsgRef::try_from(log.payload())?;
        if msg.ty() == SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID {
            out.push(SnarkAccountUpdateLogData::try_decode_log(&msg)?);
        }
    }
    Ok(out)
}

fn debug_assert_contiguous_update_ranges(
    account_updates: &BTreeMap<AccountId, Vec<AccountUpdateRecord>>,
) {
    for (account_id, records) in account_updates {
        for pair in records.windows(2) {
            debug_assert_eq!(
                pair[1].prev_next_inbox_idx(),
                pair[0].next_inbox_idx(),
                "non-contiguous snark update inbox range for account {account_id}",
            );
        }
    }
}

/// Builds an [`IndexingWrites`] payload for a DA-reconstructed epoch.
///
/// Like [`build_indexing_writes`] but with no per-block attribution and no
/// per-update state root: update records carry `update_meta: None`. The
/// post-epoch root lives on the epoch summary.
pub(crate) fn build_checkpoint_indexing_writes(
    output: &OLBlockExecutionOutput,
) -> WorkerResult<IndexingWrites> {
    let indexer_writes = output.indexer_writes();

    let created_accounts: Vec<AccountId> = indexer_writes
        .created_accounts()
        .iter()
        .map(|c| c.account_id())
        .collect();

    let snark_updates = indexer_writes.snark_state_updates();
    let snark_update_logs = collect_snark_update_logs(output.logs())?;
    if snark_updates.len() != snark_update_logs.len() {
        return Err(WorkerError::SnarkUpdateLogCountMismatch {
            expected: snark_updates.len(),
            found: snark_update_logs.len(),
        });
    }

    let mut account_updates: BTreeMap<AccountId, Vec<AccountUpdateRecord>> = BTreeMap::new();
    for (update, log) in snark_updates.iter().zip(snark_update_logs.iter()) {
        if log.new_msg_idx() != update.next_read_idx() {
            return Err(WorkerError::SnarkUpdateLogMismatch {
                expected: update.next_read_idx(),
                found: log.new_msg_idx(),
            });
        }
        let record = AccountUpdateRecord::new(
            None,
            *update.seqno().inner(),
            update.prev_next_read_idx(),
            update.next_read_idx(),
            Some(log.extra_data().to_vec()),
        );
        account_updates
            .entry(update.account_id())
            .or_default()
            .push(record);
    }

    debug_assert_contiguous_update_ranges(&account_updates);

    let mut account_inbox_writes: BTreeMap<AccountId, Vec<InboxMessageRecord>> = BTreeMap::new();
    for write in indexer_writes.inbox_messages() {
        let entry_bytes = write.entry().as_ssz_bytes();
        let record = InboxMessageRecord::new(entry_bytes, None);
        account_inbox_writes
            .entry(write.account_id())
            .or_default()
            .push(record);
    }

    Ok(IndexingWrites::new(
        created_accounts,
        account_updates,
        account_inbox_writes,
    ))
}

/// Mirrors all MMR writes from an accepted OL execution output into the proof index.
pub(crate) fn index_mmr_writes(
    mmr_index_mgr: &MmrIndexManager,
    output: &OLBlockExecutionOutput,
) -> WorkerResult<()> {
    index_inbox_mmr_writes(mmr_index_mgr, output)?;
    index_l1_block_ref_mmr_writes(mmr_index_mgr, output)?;
    Ok(())
}

/// Applies snark inbox writes to the MMR proof index.
///
/// The OL state itself stores the compact MMR root/peaks. Block assembly needs
/// historical nodes from [`MmrIndexManager`] to generate proofs for later snark
/// account updates, so the chain worker mirrors each accepted inbox append into
/// the proof index. The operation is idempotent for crash-restart retries.
pub(crate) fn index_inbox_mmr_writes(
    mmr_index_mgr: &MmrIndexManager,
    output: &OLBlockExecutionOutput,
) -> WorkerResult<()> {
    for write in output.indexer_writes().inbox_messages() {
        let expected_hash: Hash =
            <MessageEntry as TreeHash>::tree_hash_root::<Sha256Hasher>(write.entry()).into();
        let entry_bytes = write.entry().as_ssz_bytes();
        let account_id = write.account_id();
        let idx = write.index();
        let handle = mmr_index_mgr.get_handle(MmrId::SnarkMsgInbox(account_id));
        handle
            .idempotent_append_leaf_with_preimage_blocking(idx, expected_hash, entry_bytes)
            .inspect_err(|e| {
                error!(%account_id, idx, %e, "snark inbox MMR write failed");
            })?;
    }

    Ok(())
}

/// Seeds the L1 block refs MMR mirror with sentinel leaves for indices
/// `0..=genesis_l1_height`, matching the in-state MMR's genesis prefill.
///
/// Run once at chain worker initialization. Idempotent: no-op if the mirror
/// already contains the expected leaves (crash-restart safe).
pub(crate) fn prefill_l1_block_refs_mmr_blocking(
    mmr_index_mgr: &MmrIndexManager,
    genesis_l1_height: u64,
) -> WorkerResult<()> {
    let handle = mmr_index_mgr.get_handle(MmrId::L1BlockRefs);
    let leaf_count = handle.get_num_leaves_blocking()?;
    for expected_idx in leaf_count..=genesis_l1_height {
        let appended_idx = handle.append_leaf_blocking(MMR_SENTINEL_DUMMY_LEAF_HASH)?;
        if appended_idx != expected_idx {
            return Err(WorkerError::Unexpected(format!(
                "L1 block refs MMR prefill index mismatch: expected {expected_idx}, got {appended_idx}"
            )));
        }
    }
    Ok(())
}

/// Applies terminal-block L1 block ref writes to the MMR proof index.
///
/// The in-state accumulator stores only MMR peaks. Block assembly needs the
/// historical nodes to prove reduced `{block_hash, wtxids_root}` refs in later
/// snark-account updates, so the chain worker mirrors each accepted manifest
/// append into the DB-side proof index. The operation is idempotent for
/// crash-restart retries.
///
/// Assumes the mirror has already been seeded via
/// [`prefill_l1_block_refs_mmr_blocking`] at chain worker initialization.
fn index_l1_block_ref_mmr_writes(
    mmr_index_mgr: &MmrIndexManager,
    output: &OLBlockExecutionOutput,
) -> WorkerResult<()> {
    let handle = mmr_index_mgr.get_handle(MmrId::L1BlockRefs);
    for write in output.indexer_writes().l1_block_records() {
        let expected_idx = write.height as u64;
        let l1_block_ref = &write.record;
        let expected_hash: Hash = l1_block_ref.leaf_hash().into();
        let preimage = l1_block_ref.as_ssz_bytes();

        handle
            .idempotent_append_leaf_with_preimage_blocking(expected_idx, expected_hash, preimage)
            .inspect_err(|e| {
                error!(idx = expected_idx, %e, "L1 block refs MMR write failed");
            })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_acct_types::{AccountSerial, BitcoinAmount, L1BlockRecord, MsgPayload};
    use strata_db_store_sled::{MmrIndexDb, SledDbConfig};
    use strata_identifiers::Buf32;
    use strata_ol_state_support_types::{
        InboxMessageWrite, IndexerWrites, L1BlockRecordWrite, SnarkAcctStateUpdate,
    };
    use strata_snark_acct_types::Seqno;

    use super::*;

    fn setup_mmr_index_manager() -> MmrIndexManager {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = Arc::new(typed_sled::SledDb::new(db).unwrap());
        let mmr_db = Arc::new(MmrIndexDb::new(sled_db, SledDbConfig::test()).unwrap());
        MmrIndexManager::new(strata_storage::test_runtime_handle(), mmr_db)
    }

    fn message_entry(source_seed: u8, value_sats: u64) -> MessageEntry {
        let payload =
            MsgPayload::from_bytes(BitcoinAmount::from_sat(value_sats), vec![source_seed])
                .expect("message payload bytes must fit within SSZ max length");
        MessageEntry::new(AccountId::from([source_seed; 32]), 0, payload)
    }

    fn output_with_inbox_messages(
        writes: impl IntoIterator<Item = (AccountId, MessageEntry, u64)>,
    ) -> OLBlockExecutionOutput {
        let mut indexer_writes = IndexerWrites::new();
        for (account_id, entry, index) in writes {
            indexer_writes.push_inbox_message(InboxMessageWrite::new(account_id, entry, index));
        }

        OLBlockExecutionOutput::new(Buf32::zero(), WriteBatch::default(), indexer_writes, vec![])
    }

    #[test]
    fn test_build_indexing_writes_sources_extra_data_from_logs() {
        let account_id = AccountId::from([7u8; 32]);
        let serial = AccountSerial::from(7u32);
        let state = Hash::from([9u8; 32]);
        let next_read_idx = 3u64;
        let seqno = Seqno::from(5);
        let extra = vec![0xaau8, 0xbb, 0xcc];

        let mut indexer_writes = IndexerWrites::new();
        indexer_writes.push_snark_acct_update(SnarkAcctStateUpdate::new(
            account_id,
            Some(state),
            0,
            next_read_idx,
            seqno,
        ));

        // The matching log carries the extra_data that must end up in the index record.
        let log_data = SnarkAccountUpdateLogData::new(next_read_idx, extra.clone()).unwrap();
        let log = OLLog::new(serial, log_data.encode_log().unwrap());

        let output = OLBlockExecutionOutput::new(
            Buf32::zero(),
            WriteBatch::default(),
            indexer_writes,
            vec![log],
        );

        let writes = build_indexing_writes(OLBlockCommitment::null(), &output).unwrap();

        let records = writes
            .account_updates()
            .get(&account_id)
            .expect("account update should be present");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].extra_data(), Some(extra.as_slice()));
        assert_eq!(records[0].prev_next_inbox_idx(), 0);
        assert_eq!(records[0].next_inbox_idx(), next_read_idx);
    }

    #[test]
    fn test_build_indexing_writes_rejects_missing_log() {
        // A tracked snark update with no corresponding emitted log is a correlation failure.
        let mut indexer_writes = IndexerWrites::new();
        indexer_writes.push_snark_acct_update(SnarkAcctStateUpdate::new(
            AccountId::from([1u8; 32]),
            Some(Hash::from([0u8; 32])),
            0,
            0,
            Seqno::from(1),
        ));

        let output = OLBlockExecutionOutput::new(
            Buf32::zero(),
            WriteBatch::default(),
            indexer_writes,
            vec![],
        );

        let err = build_indexing_writes(OLBlockCommitment::null(), &output).unwrap_err();
        assert!(matches!(
            err,
            WorkerError::SnarkUpdateLogCountMismatch {
                expected: 1,
                found: 0
            }
        ));
    }

    fn output_with_l1_block_records(
        writes: impl IntoIterator<Item = L1BlockRecordWrite>,
    ) -> OLBlockExecutionOutput {
        let mut indexer_writes = IndexerWrites::new();
        for write in writes {
            indexer_writes.push_l1_block_record(write);
        }

        OLBlockExecutionOutput::new(Buf32::zero(), WriteBatch::default(), indexer_writes, vec![])
    }

    fn output_with_mmr_writes(
        inbox_writes: impl IntoIterator<Item = (AccountId, MessageEntry, u64)>,
        l1_writes: impl IntoIterator<Item = L1BlockRecordWrite>,
    ) -> OLBlockExecutionOutput {
        let mut indexer_writes = IndexerWrites::new();
        for (account_id, entry, index) in inbox_writes {
            indexer_writes.push_inbox_message(InboxMessageWrite::new(account_id, entry, index));
        }
        for write in l1_writes {
            indexer_writes.push_l1_block_record(write);
        }

        OLBlockExecutionOutput::new(Buf32::zero(), WriteBatch::default(), indexer_writes, vec![])
    }

    fn l1_block_record_write(height: u32, seed: u8) -> L1BlockRecordWrite {
        let record = L1BlockRecord::new([seed; 32], [seed.wrapping_add(1); 32]);
        L1BlockRecordWrite { height, record }
    }

    fn assert_mmr_entry(
        mmr_index_mgr: &MmrIndexManager,
        account_id: AccountId,
        index: u64,
        entry: &MessageEntry,
    ) {
        let handle = mmr_index_mgr.get_handle(MmrId::SnarkMsgInbox(account_id));
        let expected_hash: Hash =
            <MessageEntry as TreeHash>::tree_hash_root::<Sha256Hasher>(entry).into();

        assert_eq!(
            handle.get_leaf_blocking(index).unwrap(),
            Some(expected_hash)
        );
        assert_eq!(handle.get_blocking(index).unwrap(), entry.as_ssz_bytes());
    }

    fn assert_l1_block_ref_entry(
        mmr_index_mgr: &MmrIndexManager,
        index: u64,
        write: &L1BlockRecordWrite,
    ) {
        let handle = mmr_index_mgr.get_handle(MmrId::L1BlockRefs);
        let l1_block_ref = &write.record;
        let expected_hash: Hash = l1_block_ref.leaf_hash().into();
        let expected_preimage = l1_block_ref.as_ssz_bytes();

        assert_eq!(
            handle.get_leaf_blocking(index).unwrap(),
            Some(expected_hash)
        );
        assert_eq!(handle.get_blocking(index).unwrap(), expected_preimage);
    }

    #[test]
    fn index_mmr_writes_stores_inbox_and_l1_block_refs() {
        let mmr_index_mgr = setup_mmr_index_manager();
        let account_id = AccountId::from([1u8; 32]);
        let entry = message_entry(10, 100);
        let l1_write = l1_block_record_write(1, 20);
        let output = output_with_mmr_writes([(account_id, entry.clone(), 0)], [l1_write.clone()]);

        prefill_l1_block_refs_mmr_blocking(&mmr_index_mgr, 0).unwrap();
        index_mmr_writes(&mmr_index_mgr, &output).unwrap();

        assert_eq!(
            mmr_index_mgr
                .get_handle(MmrId::SnarkMsgInbox(account_id))
                .get_num_leaves_blocking()
                .unwrap(),
            1
        );
        assert_eq!(
            mmr_index_mgr
                .get_handle(MmrId::L1BlockRefs)
                .get_num_leaves_blocking()
                .unwrap(),
            2
        );
        assert_mmr_entry(&mmr_index_mgr, account_id, 0, &entry);
        assert_l1_block_ref_entry(&mmr_index_mgr, 1, &l1_write);
    }

    #[test]
    fn index_mmr_writes_is_idempotent() {
        let mmr_index_mgr = setup_mmr_index_manager();
        let account_id = AccountId::from([2u8; 32]);
        let entry = message_entry(11, 200);
        let first_l1 = l1_block_record_write(1, 30);
        let second_l1 = l1_block_record_write(2, 40);
        let output = output_with_mmr_writes(
            [(account_id, entry.clone(), 0)],
            [first_l1.clone(), second_l1.clone()],
        );

        prefill_l1_block_refs_mmr_blocking(&mmr_index_mgr, 0).unwrap();
        index_mmr_writes(&mmr_index_mgr, &output).unwrap();
        index_mmr_writes(&mmr_index_mgr, &output).unwrap();

        assert_eq!(
            mmr_index_mgr
                .get_handle(MmrId::SnarkMsgInbox(account_id))
                .get_num_leaves_blocking()
                .unwrap(),
            1
        );
        assert_eq!(
            mmr_index_mgr
                .get_handle(MmrId::L1BlockRefs)
                .get_num_leaves_blocking()
                .unwrap(),
            3
        );
        assert_mmr_entry(&mmr_index_mgr, account_id, 0, &entry);
        assert_l1_block_ref_entry(&mmr_index_mgr, 1, &first_l1);
        assert_l1_block_ref_entry(&mmr_index_mgr, 2, &second_l1);
    }

    #[test]
    fn index_inbox_mmr_writes_stores_expected_leaves_and_preimages() {
        let mmr_index_mgr = setup_mmr_index_manager();
        let account_one = AccountId::from([1u8; 32]);
        let account_two = AccountId::from([2u8; 32]);
        let entry_one = message_entry(10, 100);
        let entry_two = message_entry(11, 200);
        let entry_three = message_entry(12, 300);
        let output = output_with_inbox_messages([
            (account_one, entry_one.clone(), 0),
            (account_one, entry_two.clone(), 1),
            (account_two, entry_three.clone(), 0),
        ]);

        index_inbox_mmr_writes(&mmr_index_mgr, &output).unwrap();

        assert_eq!(
            mmr_index_mgr
                .get_handle(MmrId::SnarkMsgInbox(account_one))
                .get_num_leaves_blocking()
                .unwrap(),
            2
        );
        assert_eq!(
            mmr_index_mgr
                .get_handle(MmrId::SnarkMsgInbox(account_two))
                .get_num_leaves_blocking()
                .unwrap(),
            1
        );
        assert_mmr_entry(&mmr_index_mgr, account_one, 0, &entry_one);
        assert_mmr_entry(&mmr_index_mgr, account_one, 1, &entry_two);
        assert_mmr_entry(&mmr_index_mgr, account_two, 0, &entry_three);
    }

    #[test]
    fn index_inbox_mmr_writes_is_idempotent() {
        let mmr_index_mgr = setup_mmr_index_manager();
        let account_id = AccountId::from([3u8; 32]);
        let entry_one = message_entry(13, 400);
        let entry_two = message_entry(14, 500);
        let output = output_with_inbox_messages([
            (account_id, entry_one.clone(), 0),
            (account_id, entry_two.clone(), 1),
        ]);

        index_inbox_mmr_writes(&mmr_index_mgr, &output).unwrap();
        index_inbox_mmr_writes(&mmr_index_mgr, &output).unwrap();

        let handle = mmr_index_mgr.get_handle(MmrId::SnarkMsgInbox(account_id));
        assert_eq!(handle.get_num_leaves_blocking().unwrap(), 2);
        assert_mmr_entry(&mmr_index_mgr, account_id, 0, &entry_one);
        assert_mmr_entry(&mmr_index_mgr, account_id, 1, &entry_two);
    }

    #[test]
    fn prefill_then_index_seeds_genesis_and_stores_refs() {
        let mmr_index_mgr = setup_mmr_index_manager();
        let first_real = l1_block_record_write(1, 10);
        let output = output_with_l1_block_records([first_real.clone()]);

        prefill_l1_block_refs_mmr_blocking(&mmr_index_mgr, 0).unwrap();
        index_l1_block_ref_mmr_writes(&mmr_index_mgr, &output).unwrap();

        let handle = mmr_index_mgr.get_handle(MmrId::L1BlockRefs);
        assert_eq!(handle.get_num_leaves_blocking().unwrap(), 2);
        assert_eq!(
            handle.get_leaf_blocking(0).unwrap(),
            Some(MMR_SENTINEL_DUMMY_LEAF_HASH)
        );
        assert_l1_block_ref_entry(&mmr_index_mgr, 1, &first_real);
    }

    #[test]
    fn prefill_is_idempotent_across_repeated_calls() {
        let mmr_index_mgr = setup_mmr_index_manager();

        // Multiple invocations must not append duplicate sentinels.
        prefill_l1_block_refs_mmr_blocking(&mmr_index_mgr, 3).unwrap();
        prefill_l1_block_refs_mmr_blocking(&mmr_index_mgr, 3).unwrap();

        let handle = mmr_index_mgr.get_handle(MmrId::L1BlockRefs);
        assert_eq!(handle.get_num_leaves_blocking().unwrap(), 4);
    }

    #[test]
    fn index_l1_block_ref_mmr_writes_is_idempotent() {
        let mmr_index_mgr = setup_mmr_index_manager();
        let first_real = l1_block_record_write(1, 10);
        let second_real = l1_block_record_write(2, 20);
        let output = output_with_l1_block_records([first_real.clone(), second_real.clone()]);

        prefill_l1_block_refs_mmr_blocking(&mmr_index_mgr, 0).unwrap();
        index_l1_block_ref_mmr_writes(&mmr_index_mgr, &output).unwrap();
        index_l1_block_ref_mmr_writes(&mmr_index_mgr, &output).unwrap();

        let handle = mmr_index_mgr.get_handle(MmrId::L1BlockRefs);
        assert_eq!(handle.get_num_leaves_blocking().unwrap(), 3);
        assert_l1_block_ref_entry(&mmr_index_mgr, 1, &first_real);
        assert_l1_block_ref_entry(&mmr_index_mgr, 2, &second_real);
    }

    #[test]
    fn index_l1_block_ref_mmr_writes_rejects_existing_hash_mismatch() {
        let mmr_index_mgr = setup_mmr_index_manager();
        let first_real = l1_block_record_write(1, 10);
        let conflicting_first_real = l1_block_record_write(1, 11);

        prefill_l1_block_refs_mmr_blocking(&mmr_index_mgr, 0).unwrap();
        index_l1_block_ref_mmr_writes(&mmr_index_mgr, &output_with_l1_block_records([first_real]))
            .unwrap();
        let err = index_l1_block_ref_mmr_writes(
            &mmr_index_mgr,
            &output_with_l1_block_records([conflicting_first_real]),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            WorkerError::Database(DbError::MmrLeafHashMismatch { idx: 1, .. })
        ));
    }
}
