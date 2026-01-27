//! Mempool service state management.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::{Debug, Formatter},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use strata_acct_types::AccountId;
use strata_db_types::types::MempoolTxData;
use strata_identifiers::{OLBlockCommitment, OLTxId};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::{OLBlock, OLTransaction, TransactionPayload};
use strata_ol_state_types::StateProvider;
use strata_service::ServiceState;
use strata_storage::{NodeStorage, OLStateManager};
use tracing::warn;

use crate::{
    MempoolTxInvalidReason, OLMempoolError, OLMempoolResult,
    types::{
        MempoolEntry, MempoolOrderingKey, OLMempoolConfig, OLMempoolRejectReason, OLMempoolStats,
        OLMempoolTransaction,
    },
    validation::validate_transaction,
};

/// Per-account mempool state tracking.
///
/// Efficiently tracks all transactions from an account currently in the mempool,
/// along with sequence number information for validation.
#[derive(Debug, Clone, Default)]
pub(crate) struct AccountMempoolState {
    /// Set of transaction IDs from this account in the mempool.
    pub(crate) txids: BTreeSet<OLTxId>,

    /// Sequence numbers for [`SnarkAccountUpdate`](strata_snark_acct_types::SnarkAccountUpdate)
    /// transactions from this account. Used for range queries (min/max) and gap detection.
    /// Empty for accounts with only
    /// [`GenericAccountMessage`](crate::types::OLMempoolTxPayload::GenericAccountMessage)
    /// transactions.
    pub(crate) seq_nos: BTreeSet<u64>,
}

impl AccountMempoolState {
    /// Returns the sequence number range (min, max) if there are any sequence numbers.
    pub(crate) fn seq_no_range(&self) -> Option<(u64, u64)> {
        self.seq_nos
            .first()
            .zip(self.seq_nos.last())
            .map(|(&min, &max)| (min, max))
    }
}

/// Immutable context for mempool service (shared via Arc).
#[derive(Clone)]
pub(crate) struct MempoolContext<P: StateProvider> {
    /// Mempool configuration.
    pub(crate) config: OLMempoolConfig,

    /// Storage backend for database operations (transactions, blocks).
    pub(crate) storage: Arc<NodeStorage>,

    /// State provider for fetching OL state at different chain tips.
    pub(crate) provider: Arc<P>,
}

impl MempoolContext<OLStateManager> {
    /// Create new mempool context.
    ///
    /// Extracts the state provider from storage.
    pub(crate) fn new(config: OLMempoolConfig, storage: Arc<NodeStorage>) -> Self {
        let provider = storage.ol_state().clone();
        Self {
            config,
            storage,
            provider,
        }
    }

    /// Create new mempool context with explicit provider.
    pub(crate) fn new_with_provider<P: StateProvider>(
        config: OLMempoolConfig,
        storage: Arc<NodeStorage>,
        provider: Arc<P>,
    ) -> MempoolContext<P> {
        MempoolContext {
            config,
            storage,
            provider,
        }
    }
}

impl<P: StateProvider> Debug for MempoolContext<P> {
    #[expect(clippy::absolute_paths, reason = "qualified Result avoids ambiguity")]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MempoolContext")
            .field("config", &self.config)
            .finish()
    }
}

/// Combined state for the service (context + mutable state).
///
/// # Type Parameters
///
/// - `P`: The state provider type that implements [`StateProvider`]. This enables production use
///   with database-backed state and fast in-memory testing.
#[derive(Debug)]
pub(crate) struct MempoolServiceState<P: StateProvider> {
    ctx: Arc<MempoolContext<P>>,

    /// In-memory entries indexed by transaction ID.
    entries: HashMap<OLTxId, MempoolEntry>,

    /// Ordering index: MempoolOrderingKey → transaction ID.
    ordering_index: BTreeMap<MempoolOrderingKey, OLTxId>,

    /// Per-account mempool state.
    /// Tracks all txids and sequence numbers for each account.
    account_state: HashMap<AccountId, AccountMempoolState>,

    /// State accessor for validation. Updated when chain tip changes.
    state_accessor: Arc<P::State>,

    /// Mempool statistics.
    stats: OLMempoolStats,
}

impl<P: StateProvider> MempoolServiceState<P> {
    /// Create new mempool service state.
    #[expect(dead_code, reason = "another constructor is used")]
    pub(crate) async fn new(
        config: OLMempoolConfig,
        storage: Arc<NodeStorage>,
        provider: Arc<P>,
        tip: OLBlockCommitment,
    ) -> OLMempoolResult<Self> {
        let ctx = Arc::new(MempoolContext::new_with_provider(config, storage, provider));
        Self::new_with_context(ctx, tip).await
    }

    /// Create new mempool service state with an existing context.
    /// Used for testing.
    ///
    /// Fetches the state for the given tip from the provider.
    pub(crate) async fn new_with_context(
        ctx: Arc<MempoolContext<P>>,
        tip: OLBlockCommitment,
    ) -> OLMempoolResult<Self> {
        let state_accessor = ctx
            .provider
            .get_state_for_tip_async(tip)
            .await
            .map_err(|e| {
                OLMempoolError::StateProvider(format!(
                    "Failed to get state for tip {:?}: {}",
                    tip, e
                ))
            })?
            .ok_or_else(|| {
                OLMempoolError::StateProvider(format!("State not found for tip {:?}", tip))
            })?;

        Ok(Self {
            ctx,
            entries: HashMap::new(),
            ordering_index: BTreeMap::new(),
            account_state: HashMap::new(),
            state_accessor,
            stats: OLMempoolStats::default(),
        })
    }

    /// Load existing transactions from database.
    ///
    /// Deserializes and validates each transaction. Invalid transactions are skipped and
    /// removed from the database.
    pub(crate) async fn load_from_db(&mut self) -> OLMempoolResult<()> {
        let mut all_txs = self.ctx.storage.mempool().get_all_txs()?;

        // Sort by `timestamp_micros` to validate transactions in order
        all_txs.sort_by_key(|tx_data| tx_data.timestamp_micros);

        let mut loaded_count = 0;
        let mut skipped_count = 0;

        for tx_data in all_txs {
            // Parse transaction from bytes
            let tx: OLMempoolTransaction = match ssz::Decode::from_ssz_bytes(&tx_data.tx_bytes) {
                Ok(tx) => tx,
                Err(e) => {
                    // Skip malformed transaction and remove from DB
                    warn!(
                        ?tx_data.txid,
                        ?e,
                        "Skipping malformed transaction from database"
                    );
                    let _ = self.ctx.storage.mempool().del_tx(tx_data.txid);
                    skipped_count += 1;
                    continue;
                }
            };

            let txid = tx_data.txid;

            // Validate transaction
            // Note: this plays nice with sequence number validation because we don't allow gaps
            // (sequence numbers and timestamps are guaranteed to be compatible). When we move to a
            // different priority ordering, this should be revised.
            if let Err(e) =
                validate_transaction(txid, &tx, &self.state_accessor, &self.account_state)
            {
                // Skip invalid transaction and remove from DB
                warn!(
                    ?tx_data.txid,
                    ?e,
                    "Skipping invalid transaction from database"
                );
                let _ = self.ctx.storage.mempool().del_tx(tx_data.txid);

                // Update reject stats if this is a trackable rejection reason
                if let Some(reason) = OLMempoolRejectReason::from_error(&e) {
                    self.update_stats_on_reject(reason);
                }

                skipped_count += 1;
                continue;
            }

            // Create entry using stored timestamp from database
            let ordering_key = MempoolOrderingKey::for_transaction(&tx, tx_data.timestamp_micros);
            let tx_size = tx_data.tx_bytes.len();
            let entry = MempoolEntry::new(tx, ordering_key, tx_size);

            // Add to in-memory state (already in DB, so no write needed)
            self.add_tx_to_in_memory_state(txid, entry);

            loaded_count += 1;
        }

        if skipped_count > 0 {
            tracing::info!(
                loaded_count,
                skipped_count,
                "Loaded transactions from database"
            );
        }

        Ok(())
    }

    /// Handle submit transaction command.
    pub(crate) async fn handle_submit_transaction(
        &mut self,
        tx: Box<OLMempoolTransaction>,
    ) -> OLMempoolResult<OLTxId> {
        // Add to mempool
        let txid = self.add_transaction(*tx).await?;
        Ok(txid)
    }

    /// Handle get transactions command (returns transactions in priority order).
    pub(crate) async fn handle_get_transactions(
        &mut self,
        limit: usize,
    ) -> OLMempoolResult<Vec<(OLTxId, OLMempoolTransaction)>> {
        // Gap checking at submission ensures no gaps exist.
        // Simply return transactions in priority order.
        let result: Vec<(OLTxId, OLMempoolTransaction)> = self
            .ordering_index
            .values()
            .take(limit)
            .filter_map(|txid| {
                let entry = self.entries.get(txid)?;
                Some((*txid, entry.tx.clone()))
            })
            .collect();

        Ok(result)
    }

    /// Handle report invalid transactions command.
    pub(crate) fn handle_report_invalid_transactions(
        &mut self,
        txs: Vec<(OLTxId, MempoolTxInvalidReason)>,
    ) {
        let remove_ids: Vec<_> = txs
            .into_iter()
            .filter_map(|(txid, reason)| should_remove_tx(reason).then_some(txid))
            .collect();
        self.remove_transactions(&remove_ids, true);
    }

    /// Check if transaction exists in mempool.
    pub(crate) fn contains(&self, id: &OLTxId) -> bool {
        self.entries.contains_key(id)
    }

    /// Get mempool statistics.
    pub(crate) fn stats(&self) -> &OLMempoolStats {
        &self.stats
    }

    /// Check mempool capacity limits before adding a transaction.
    ///
    /// Returns appropriate error if any limit would be exceeded.
    fn check_capacity_limits(&self, tx_size: usize) -> OLMempoolResult<()> {
        // Check transaction count limit
        if self.entries.len() >= self.ctx.config.max_tx_count {
            return Err(OLMempoolError::MempoolFull {
                current: self.entries.len(),
                limit: self.ctx.config.max_tx_count,
            });
        }

        // Check total mempool byte size limit
        if self.stats.total_bytes() + tx_size > self.ctx.config.max_mempool_bytes {
            return Err(OLMempoolError::MempoolByteLimitExceeded {
                current: self.stats.total_bytes() + tx_size,
                limit: self.ctx.config.max_mempool_bytes,
            });
        }

        Ok(())
    }

    /// Find account transaction with the same sequence number.
    ///
    /// Returns txid if this is a
    /// [`SnarkAccountUpdate`](strata_snark_acct_types::SnarkAccountUpdate) with a sequence number
    /// that already exists in the mempool for the target account. Returns None otherwise.
    fn find_account_tx_with_same_seqno(&self, tx: &OLMempoolTransaction) -> Option<OLTxId> {
        let base_update = tx.base_update()?;
        let target_account = tx.target();
        let tx_seq_no = base_update.operation().seq_no();

        // Get account state and check if seq_no exists
        let acct_state = self.account_state.get(&target_account)?;
        let (min_seq_no, max_seq_no) = acct_state.seq_no_range()?;

        if tx_seq_no < min_seq_no || tx_seq_no > max_seq_no {
            return None;
        }

        // Find txid with this seq_no in the account's transactions
        for txid in &acct_state.txids {
            if let Some(entry) = self.entries.get(txid) {
                if let Some(entry_update) = entry.tx.base_update()
                    && entry_update.operation().seq_no() == tx_seq_no
                {
                    return Some(*txid);
                }
            } else {
                // Data integrity issue: txid in account_state but missing from entries
                warn!(
                    ?txid,
                    ?target_account,
                    "txid exists in account_state but missing from entries"
                );
            }
        }
        None
    }

    /// Add a transaction to the mempool.
    ///
    /// Returns the transaction ID. Idempotent - returns existing txid if duplicate.
    pub(crate) async fn add_transaction(
        &mut self,
        tx: OLMempoolTransaction,
    ) -> OLMempoolResult<OLTxId> {
        let txid = tx.compute_txid();

        // Idempotent check - if already present, return success
        if self.contains(&txid) {
            self.update_stats_on_reject(OLMempoolRejectReason::Duplicate);
            return Ok(txid);
        }

        // Encode transaction once for both size validation and database persistence
        let tx_bytes = ssz::Encode::as_ssz_bytes(&tx);
        let tx_size = tx_bytes.len();

        // Check individual transaction size limit
        if tx_size > self.ctx.config.max_tx_size {
            self.update_stats_on_reject(OLMempoolRejectReason::TransactionTooLarge);
            return Err(OLMempoolError::TransactionTooLarge {
                size: tx_size,
                limit: self.ctx.config.max_tx_size,
            });
        }

        // Check mempool capacity limits
        if let Err(e) = self.check_capacity_limits(tx_size) {
            self.update_stats_on_reject(OLMempoolRejectReason::MempoolFull);
            return Err(e);
        }

        // Validate transaction using STF validation helpers
        // This checks: slot bounds, account existence, sequence number validity
        validate_transaction(txid, &tx, &self.state_accessor, &self.account_state)?;

        // Check if this is a replacement transaction and remove old one if needed
        if let Some(old_txid) = self.find_account_tx_with_same_seqno(&tx) {
            // Remove old tx with same sequence number (last-write-wins), no cascade
            self.remove_transactions(&[old_txid], false);
        }

        // Generate timestamp for ordering
        let timestamp_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX epoch")
            .as_micros() as u64;

        let ordering_key = MempoolOrderingKey::for_transaction(&tx, timestamp_micros);
        let entry = MempoolEntry::new(tx.clone(), ordering_key, tx_size);

        // Persist to database first
        let tx_data = MempoolTxData::new(txid, tx_bytes, timestamp_micros);
        self.ctx.storage.mempool().put_tx(tx_data)?;

        // Add to in-memory state
        self.add_tx_to_in_memory_state(txid, entry);

        Ok(txid)
    }

    /// Add a transaction to in-memory state.
    ///
    /// Updates all in-memory data structures:
    /// - entries: Main transaction storage
    /// - ordering_index: Priority queue for ordering
    /// - account_state: Per-account tracking for validation
    ///
    /// Also updates statistics. Does NOT write to database or perform validation.
    fn add_tx_to_in_memory_state(&mut self, txid: OLTxId, entry: MempoolEntry) {
        let ordering_key = entry.ordering_key;
        let target_account = entry.tx.target();
        let tx_size = entry.size_bytes;

        // Add to ordering index
        self.ordering_index.insert(ordering_key, txid);

        // Add to entries
        self.entries.insert(txid, entry.clone());

        // Add to account_state index
        let acct_state = self.account_state.entry(target_account).or_default();
        acct_state.txids.insert(txid);

        // Add seq_no if this is a SnarkAccountUpdate
        if let Some(base_update) = entry.tx.base_update() {
            let seq_no = base_update.operation().seq_no();
            acct_state.seq_nos.insert(seq_no);
        }

        // Update stats
        self.update_stats_on_add(tx_size);
    }

    /// Helper to remove a single transaction from all internal data structures.
    fn remove_single_tx(&mut self, txid: OLTxId, entry: &MempoolEntry) -> OLMempoolResult<()> {
        // Remove from database first
        self.ctx.storage.mempool().del_tx(txid)?;

        // Get ordering key for ordering index removal
        let ordering_key = entry.ordering_key;
        let size_bytes = entry.size_bytes;
        let account_id = entry.tx.target();

        // Remove from memory
        self.entries.remove(&txid);
        self.ordering_index.remove(&ordering_key);

        // Remove from account_state index
        if let Some(acct_state) = self.account_state.get_mut(&account_id) {
            acct_state.txids.remove(&txid);

            // Remove seq_no if this is a SnarkAccountUpdate
            if let Some(base_update) = entry.tx.base_update() {
                let seq_no = base_update.operation().seq_no();
                acct_state.seq_nos.remove(&seq_no);
            }

            // Remove account state entirely if no more transactions
            if acct_state.txids.is_empty() {
                self.account_state.remove(&account_id);
            }
        }

        // Update stats
        self.update_stats_on_remove(size_bytes);

        Ok(())
    }

    /// Remove transactions from the mempool.
    ///
    /// If `cascade` is true, also removes dependent transactions (same account, higher seq_no).
    /// Returns the IDs of all removed transactions.
    fn remove_transactions(&mut self, ids: &[OLTxId], cascade: bool) -> Vec<OLTxId> {
        let Ok((mut removed, account_min_seq)) = self.remove_txs_internal(ids) else {
            return Vec::new();
        };

        if cascade {
            for (account, min_failed_seq) in account_min_seq {
                let _ = self.cascade_remove_for_account(account, min_failed_seq, &mut removed);
            }
        }

        removed
    }

    /// Internal helper to remove specified transactions and collect account tracking info.
    ///
    /// Returns (removed_txids, account_min_seq) where account_min_seq contains the minimum
    /// seq_no per account for cascade removal.
    fn remove_txs_internal(
        &mut self,
        ids: &[OLTxId],
    ) -> OLMempoolResult<(Vec<OLTxId>, HashMap<AccountId, u64>)> {
        let mut removed = Vec::with_capacity(ids.len());
        let mut account_min_seq: HashMap<AccountId, u64> = HashMap::new();

        for txid in ids {
            if let Some(entry) = self.entries.get(txid).cloned() {
                let account = entry.tx.target();

                // Remove using helper
                self.remove_single_tx(*txid, &entry)?;

                // Track minimum seq_no for account (for cascade removal)
                if let Some(base_update) = entry.tx.base_update() {
                    let seq_no = base_update.operation().seq_no();
                    account_min_seq
                        .entry(account)
                        .and_modify(|min_seq| *min_seq = (*min_seq).min(seq_no))
                        .or_insert(seq_no);
                } else {
                    // For GenericAccountMessage, just track the account with sentinel value
                    account_min_seq.entry(account).or_insert(u64::MAX);
                }

                removed.push(*txid);
            }
        }

        Ok((removed, account_min_seq))
    }

    /// Helper: Cascade-remove transactions for an account starting from minimum failed seq_no.
    ///
    /// Removes all transactions with seq_no >= min_failed_seq.
    /// Since we enforce no gaps in sequence numbers, max_remaining_seq = min_failed_seq - 1.
    fn cascade_remove_for_account(
        &mut self,
        account: AccountId,
        min_failed_seq: u64,
        removed: &mut Vec<OLTxId>,
    ) -> OLMempoolResult<()> {
        // Use account_state index to iterate only this account's transactions
        let Some(acct_state) = self.account_state.get(&account) else {
            return Ok(()); // No transactions for this account
        };

        // Collect txids to remove
        let to_remove: Vec<OLTxId> = acct_state
            .txids
            .iter()
            .filter_map(|&txid| {
                let entry = self.entries.get(&txid)?;
                let base_update = entry.tx.base_update()?;
                let seq_no = base_update.operation().seq_no();
                (seq_no >= min_failed_seq).then_some(txid)
            })
            .collect();

        // Remove and add to removed list
        for txid in to_remove {
            // Get entry - this should always succeed since we just collected txids from entries
            let Some(entry) = self.entries.get(&txid).cloned() else {
                // Data integrity issue: txid collected from entries but now missing
                // This should never happen in single-threaded execution
                warn!(?txid, "txid collected for removal but missing from entries");
                continue;
            };

            // Remove using helper
            self.remove_single_tx(txid, &entry)?;

            removed.push(txid);
        }

        Ok(())
    }

    /// Extracts transaction IDs grouped by account from a block.
    ///
    /// Returns HashMap where keys are accounts with included transactions
    /// and values are the transaction IDs for that account.
    ///
    /// Converts block transactions to mempool format (without accumulator proofs).
    fn extract_account_txs_from_block(block: &OLBlock) -> HashMap<AccountId, Vec<OLTxId>> {
        let mut by_account: HashMap<AccountId, Vec<OLTxId>> = HashMap::new();

        if let Some(tx_segment) = block.body().tx_segment() {
            for tx in tx_segment.txs() {
                // Convert to mempool transaction (removes accumulator proofs)
                if let Ok(mempool_tx) = Self::convert_block_tx_to_mempool_tx(tx) {
                    let account = mempool_tx.target();
                    let txid = mempool_tx.compute_txid();
                    by_account.entry(account).or_default().push(txid);
                }
            }
        }

        by_account
    }

    /// Converts a block transaction to a mempool transaction by removing accumulator proofs.
    ///
    /// For [`SnarkAccountUpdate`](strata_snark_acct_types::SnarkAccountUpdate) transactions, this
    /// extracts only the base_update without accumulator_proofs. For
    /// [`GenericAccountMessage`](crate::types::OLMempoolTxPayload::GenericAccountMessage)
    /// transactions, this is a direct conversion.
    fn convert_block_tx_to_mempool_tx(
        block_tx: &OLTransaction,
    ) -> Result<OLMempoolTransaction, OLMempoolError> {
        let attachment = block_tx.attachment().clone();
        match block_tx.payload() {
            TransactionPayload::GenericAccountMessage(gam) => {
                OLMempoolTransaction::new_generic_account_message(
                    *gam.target(),
                    gam.payload().to_vec(),
                    attachment,
                )
                .map_err(|e| OLMempoolError::Serialization(e.to_string()))
            }
            TransactionPayload::SnarkAccountUpdate(snark_payload) => {
                let target = *snark_payload.target();
                let base_update = snark_payload.update_container().base_update().clone();
                Ok(OLMempoolTransaction::new_snark_account_update(
                    target,
                    base_update,
                    attachment,
                ))
            }
        }
    }

    /// Update stats when a transaction is added successfully.
    fn update_stats_on_add(&mut self, tx_size: usize) {
        self.stats.mempool_size += 1;
        self.stats.total_bytes += tx_size;
        self.stats.enqueues_accepted += 1;
    }

    /// Update stats when a transaction is removed.
    fn update_stats_on_remove(&mut self, tx_size: usize) {
        self.stats.mempool_size -= 1;
        self.stats.total_bytes -= tx_size;
    }

    /// Update stats when a transaction is rejected.
    fn update_stats_on_reject(&mut self, reason: OLMempoolRejectReason) {
        self.stats.enqueues_rejected += 1;
        self.stats.rejects_by_reason.increment(reason);
    }

    /// Revalidates all transactions in the mempool against the current state.
    ///
    /// This is necessary after state changes (new block, slot change) because:
    /// - Slot bounds (`min_slot`, `max_slot`) depend on `state.cur_slot()`
    ///
    /// Returns a list of transaction IDs that failed validation.
    fn revalidate_all_transactions(&self) -> Vec<OLTxId> {
        self.entries
            .iter()
            .filter_map(|(txid, entry)| {
                validate_transaction(*txid, &entry.tx, &self.state_accessor, &self.account_state)
                    .is_err()
                    .then_some(*txid)
            })
            .collect()
    }

    /// Handles a new block: removes included transactions and revalidates remaining ones.
    ///
    /// This method:
    /// 1. Fetches the new block from OL block database
    /// 2. Extracts transaction IDs from the block
    /// 3. Removes those transactions from the mempool (they're now in a block)
    /// 4. Revalidates remaining transactions (state may have changed)
    async fn handle_new_block(&mut self, new_tip: OLBlockCommitment) -> OLMempoolResult<()> {
        // Step 1: Fetch new block from OL block database
        let block = self
            .ctx
            .storage
            .ol_block()
            .get_block_data_async(*new_tip.blkid())
            .await
            .map_err(|e| {
                OLMempoolError::AccountStateAccess(format!(
                    "Failed to get block for tip {:?}: {e}",
                    new_tip
                ))
            })?
            .ok_or_else(|| {
                OLMempoolError::AccountStateAccess(format!("Block not found for tip {:?}", new_tip))
            })?;

        // Step 2: Extract transaction IDs grouped by account
        let txids_by_account = Self::extract_account_txs_from_block(&block);

        // Step 3: Remove included transactions from mempool (no cascade)
        let included_txids: Vec<OLTxId> = txids_by_account.values().flatten().copied().collect();
        if !included_txids.is_empty() {
            self.remove_transactions(&included_txids, false);
        }

        // Step 4: Revalidate all transactions
        let invalid_txids: Vec<OLTxId> = self.revalidate_all_transactions();
        if !invalid_txids.is_empty() {
            // Remove invalid transactions with cascade (dependents are also invalid)
            self.remove_transactions(&invalid_txids, true);
        }

        Ok(())
    }

    /// Get parent block commitment by fetching block header.
    async fn get_parent_commitment(
        &self,
        commitment: OLBlockCommitment,
    ) -> OLMempoolResult<OLBlockCommitment> {
        let block = self
            .ctx
            .storage
            .ol_block()
            .get_block_data_async(*commitment.blkid())
            .await?
            .ok_or_else(|| {
                OLMempoolError::AccountStateAccess(format!(
                    "Block not found for commitment {:?}",
                    commitment
                ))
            })?;

        let parent_slot = commitment.slot() - 1;
        let parent_blkid = *block.header().parent_blkid();
        Ok(OLBlockCommitment::new(parent_slot, parent_blkid))
    }

    /// Handle chain update.
    ///
    /// 1. Load state for new tip and update state accessor
    /// 2. Walk backwards from new tip to current tip via parent links
    /// 3. Process all blocks in chronological order (oldest to newest)
    pub(crate) async fn handle_chain_update(
        &mut self,
        new_tip: OLBlockCommitment,
    ) -> OLMempoolResult<()> {
        let current_slot = self.state_accessor.cur_slot();

        // Load state for new tip from provider
        let new_state = self
            .ctx
            .provider
            .get_state_for_tip_async(new_tip)
            .await
            .map_err(|e| {
                OLMempoolError::StateProvider(format!(
                    "Failed to load state for tip {:?}: {}",
                    new_tip, e
                ))
            })?
            .ok_or_else(|| {
                OLMempoolError::StateProvider(format!("State not found for tip {:?}", new_tip))
            })?;

        // Update state accessor
        self.state_accessor = new_state;

        // Walk backwards from new tip to current tip, collecting all block commitments
        let mut blocks_to_process = Vec::new();
        let mut walk = new_tip;

        while walk.slot() > current_slot {
            blocks_to_process.push(walk);
            walk = self.get_parent_commitment(walk).await?;
        }

        // Reverse to get chronological order (oldest first)
        blocks_to_process.reverse();

        // Process each block in chronological order
        for block in blocks_to_process {
            self.handle_new_block(block).await?;
        }

        Ok(())
    }
}

impl<P: StateProvider> ServiceState for MempoolServiceState<P> {
    fn name(&self) -> &str {
        "mempool"
    }
}

/// Returns true if a transaction reported as invalid should be removed from the mempool.
fn should_remove_tx(reason: MempoolTxInvalidReason) -> bool {
    matches!(reason, MempoolTxInvalidReason::Invalid)
}

#[cfg(test)]
mod tests {
    use ssz_types::Optional;
    use strata_identifiers::Buf32;

    use super::*;
    use crate::{
        DEFAULT_COMMAND_BUFFER_SIZE, DEFAULT_MAX_MEMPOOL_BYTES, DEFAULT_MAX_REORG_DEPTH,
        test_utils::{
            create_test_account_id_with, create_test_attachment_with_slots,
            create_test_block_commitment, create_test_context, create_test_generic_tx_with_size,
            create_test_ol_state_for_tip, create_test_snark_tx_with_seq_no,
            create_test_snark_tx_with_seq_no_and_slots, create_test_state_provider,
            create_test_tx_with_id,
        },
        types::OLMempoolConfig,
    };

    #[tokio::test]
    async fn test_add_transaction() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let txid1 = state.add_transaction(tx1.clone()).await.unwrap();

        // Transaction should be in mempool
        assert!(state.contains(&txid1));
        assert_eq!(state.stats().mempool_size(), 1);

        // Idempotent - adding again should succeed
        let txid1_again = state.add_transaction(tx1).await.unwrap();
        assert_eq!(txid1, txid1_again);
        assert_eq!(state.stats().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_add_transaction_capacity_limit() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 2,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add two transactions (at capacity) - use sequential seq_no
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        state.add_transaction(tx1).await.unwrap();
        state.add_transaction(tx2).await.unwrap();

        // Third transaction should fail
        let tx3 = create_test_snark_tx_with_seq_no(3, 0);
        let result = state.add_transaction(tx3).await;
        assert!(matches!(result, Err(OLMempoolError::MempoolFull { .. })));
    }

    #[tokio::test]
    async fn test_snark_same_account_seq_no_ordering() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // SnarkAccountUpdate transactions ordered by seq_no
        // Use same account with sequential seq_no to verify seq_no ordering
        let account_id = 50;
        let snark1 = create_test_snark_tx_with_seq_no(account_id, 0);
        let snark2 = create_test_snark_tx_with_seq_no(account_id, 1);
        let snark3 = create_test_snark_tx_with_seq_no(account_id, 2);

        state.add_transaction(snark1).await.unwrap();
        state.add_transaction(snark2).await.unwrap();
        state.add_transaction(snark3).await.unwrap();

        // SnarkAccountUpdate transactions should be ordered by seq_no (0 < 1 < 2)
        let txs = state.handle_get_transactions(3).await.unwrap();
        assert_eq!(txs.len(), 3);
        // All transactions target same account, should be in seq_no order
        let tx1_seq = txs[0].1.base_update().unwrap().operation().seq_no();
        let tx2_seq = txs[1].1.base_update().unwrap().operation().seq_no();
        let tx3_seq = txs[2].1.base_update().unwrap().operation().seq_no();
        assert_eq!(tx1_seq, 0);
        assert_eq!(tx2_seq, 1);
        assert_eq!(tx3_seq, 2);
    }

    #[tokio::test]
    async fn test_gam_priority_fifo_order() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add three GAM transactions
        // They should get different priorities due to insertion order
        let account1 = create_test_account_id_with(200);
        let gam1 = OLMempoolTransaction::new_generic_account_message(
            account1,
            vec![1, 2, 3],
            create_test_attachment_with_slots(None, None),
        )
        .unwrap();
        let gam1_target = gam1.target();
        state.add_transaction(gam1).await.unwrap();

        let account2 = create_test_account_id_with(201);
        let gam2 = OLMempoolTransaction::new_generic_account_message(
            account2,
            vec![4, 5, 6],
            create_test_attachment_with_slots(None, None),
        )
        .unwrap();
        let gam2_target = gam2.target();
        state.add_transaction(gam2).await.unwrap();

        let account3 = create_test_account_id_with(202);
        let gam3 = OLMempoolTransaction::new_generic_account_message(
            account3,
            vec![7, 8, 9],
            create_test_attachment_with_slots(None, None),
        )
        .unwrap();
        let gam3_target = gam3.target();
        state.add_transaction(gam3).await.unwrap();

        // All three GAM transactions
        // Should be ordered by insertion order (FIFO)
        let txs = state.handle_get_transactions(3).await.unwrap();
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].1.target(), gam1_target); // First inserted
        assert_eq!(txs[1].1.target(), gam2_target); // Second inserted
        assert_eq!(txs[2].1.target(), gam3_target); // Third inserted
    }

    #[tokio::test]
    async fn test_snark_priority_different_accounts_same_seq_no() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add three SnarkAccountUpdate transactions from DIFFERENT accounts
        // All with seq_no=0 (valid for each account)
        // They should get different priorities
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1_target = tx1.target();
        state.add_transaction(tx1).await.unwrap();

        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        let tx2_target = tx2.target();
        state.add_transaction(tx2).await.unwrap();

        let tx3 = create_test_snark_tx_with_seq_no(3, 0);
        let tx3_target = tx3.target();
        state.add_transaction(tx3).await.unwrap();

        // All three transactions have seq_no=0 but different accounts
        // Should be ordered by insertion order (FIFO)
        let txs = state.handle_get_transactions(3).await.unwrap();
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].1.target(), tx1_target); // First inserted
        assert_eq!(txs[1].1.target(), tx2_target); // Second inserted
        assert_eq!(txs[2].1.target(), tx3_target); // Third inserted
    }

    #[tokio::test]
    async fn test_gap_rejection() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transaction with seq_no=0 for account 1
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        state.add_transaction(tx1).await.unwrap();

        // Try to add transaction with seq_no=2 (gap - missing seq_no=1)
        // Should be REJECTED
        let tx3 = create_test_snark_tx_with_seq_no(1, 2);
        let result = state.add_transaction(tx3).await;
        assert!(matches!(
            result,
            Err(OLMempoolError::SequenceNumberGap {
                expected: 1,
                actual: 2
            })
        ));

        // Mempool should still have only the first transaction
        assert_eq!(state.stats().mempool_size(), 1);

        // Now add seq_no=1 (correct sequential order)
        let tx2 = create_test_snark_tx_with_seq_no(1, 1);
        state.add_transaction(tx2).await.unwrap();

        // Now we can add seq_no=2
        let tx3_retry = create_test_snark_tx_with_seq_no(1, 2);
        state.add_transaction(tx3_retry).await.unwrap();

        // Should have 3 transactions now (0, 1, 2)
        assert_eq!(state.stats().mempool_size(), 3);
    }

    #[tokio::test]
    async fn test_get_transactions_limit() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add 5 transactions - each to different account with seq_no 0
        for i in 1..=5 {
            let tx = create_test_snark_tx_with_seq_no(i, 0);
            state.add_transaction(tx).await.unwrap();
        }

        // Request only 3
        let txs = state.handle_get_transactions(3).await.unwrap();
        assert_eq!(txs.len(), 3);
    }

    #[tokio::test]
    async fn test_snark_priority_ordering() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Create snark updates with sequential seq_nos: 0, 1, 2 for same account
        let snark1 = create_test_snark_tx_with_seq_no(1, 0);
        let snark1_target = snark1.target();

        let snark2 = create_test_snark_tx_with_seq_no(1, 1);
        let snark2_target = snark2.target();

        let snark3 = create_test_snark_tx_with_seq_no(1, 2);
        let snark3_target = snark3.target();

        // Add transactions with seq_no 0, 1, 2
        state.add_transaction(snark1).await.unwrap();
        state.add_transaction(snark2).await.unwrap();
        state.add_transaction(snark3).await.unwrap();

        // SnarkAccountUpdate transactions should be ordered by seq_no (0 < 1 < 2)
        let txs = state.handle_get_transactions(3).await.unwrap();
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].1.target(), snark1_target); // seq_no 0
        assert_eq!(txs[1].1.target(), snark2_target); // seq_no 1
        assert_eq!(txs[2].1.target(), snark3_target); // seq_no 2
    }

    #[tokio::test]
    async fn test_remove_transactions() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions - each to different account with seq_no 0
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        let txid1 = state.add_transaction(tx1.clone()).await.unwrap();
        let txid2 = state.add_transaction(tx2.clone()).await.unwrap();

        assert_eq!(state.stats().mempool_size(), 2);

        // Remove one transaction (no cascade)
        let removed = state.remove_transactions(&[txid1], false);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], txid1);

        // Should be gone
        assert!(!state.contains(&txid1));
        assert!(state.contains(&txid2));
        assert_eq!(state.stats().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_remove_nonexistent_transaction() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Remove transaction that doesn't exist - should succeed with empty result
        let fake_txid = OLTxId::from(Buf32::from([0u8; 32]));
        let removed = state.remove_transactions(&[fake_txid], false);
        assert_eq!(removed.len(), 0);
    }

    #[tokio::test]
    async fn test_load_from_db() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context.clone(), tip)
            .await
            .unwrap();

        // Add transactions - mix of different accounts and sequential txs for same account
        let account1 = create_test_account_id_with(1);
        let account2 = create_test_account_id_with(2);

        let tx1 = create_test_snark_tx_with_seq_no(1, 0); // Account1, seq_no=0
        let tx2 = create_test_snark_tx_with_seq_no(2, 0); // Account2, seq_no=0
        let tx3 = create_test_snark_tx_with_seq_no(1, 1); // Account1, seq_no=1
        let tx4 = create_test_snark_tx_with_seq_no(1, 2); // Account1, seq_no=2

        let txid1 = state.add_transaction(tx1).await.unwrap();
        let txid2 = state.add_transaction(tx2).await.unwrap();
        let txid3 = state.add_transaction(tx3).await.unwrap();
        let txid4 = state.add_transaction(tx4).await.unwrap();

        // Create new state and load from DB
        let mut state2 = MempoolServiceState::new_with_context(context.clone(), tip)
            .await
            .unwrap();
        state2.load_from_db().await.unwrap();

        // Should have 4 transactions
        assert_eq!(state2.stats().mempool_size(), 4);

        // Verify all 4 txids are present
        assert!(state2.entries.contains_key(&txid1));
        assert!(state2.entries.contains_key(&txid2));
        assert!(state2.entries.contains_key(&txid3));
        assert!(state2.entries.contains_key(&txid4));

        // Verify account1 has all three seq_nos [0, 1, 2] in account_state
        let acct1_state = state2.account_state.get(&account1).unwrap();
        assert_eq!(acct1_state.txids.len(), 3);
        let (min_seq, max_seq) = acct1_state.seq_no_range().unwrap();
        assert_eq!(min_seq, 0);
        assert_eq!(max_seq, 2);

        // Verify account2 has seq_no [0]
        let acct2_state = state2.account_state.get(&account2).unwrap();
        assert_eq!(acct2_state.txids.len(), 1);
        let (min_seq, max_seq) = acct2_state.seq_no_range().unwrap();
        assert_eq!(min_seq, 0);
        assert_eq!(max_seq, 0);
    }

    #[tokio::test]
    async fn test_handle_get_transactions_ordering() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 100,
            max_tx_size: 10_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add multiple transactions with different accounts
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        let tx3 = create_test_snark_tx_with_seq_no(1, 1);
        let tx4 = create_test_snark_tx_with_seq_no(3, 0);

        let txid1 = state.add_transaction(tx1).await.unwrap();
        let txid2 = state.add_transaction(tx2).await.unwrap();
        let txid3 = state.add_transaction(tx3).await.unwrap();
        let txid4 = state.add_transaction(tx4).await.unwrap();

        // Verify ordering: tx1 (id=0), tx2 (id=1), tx3 (id=2), tx4 (id=3)
        let txs = state.handle_get_transactions(10).await.unwrap();
        assert_eq!(txs.len(), 4);
        // handle_get_transactions returns Vec<(OLTxId, OLMempoolTransaction)>
        let (tx1_id_result, _) = &txs[0];
        let (tx2_id_result, _) = &txs[1];
        let (tx3_id_result, _) = &txs[2];
        let (tx4_id_result, _) = &txs[3];
        assert_eq!(*tx1_id_result, txid1);
        assert_eq!(*tx2_id_result, txid2);
        assert_eq!(*tx3_id_result, txid3);
        assert_eq!(*tx4_id_result, txid4);

        // Test limit parameter
        let txs = state.handle_get_transactions(2).await.unwrap();
        assert_eq!(txs.len(), 2);
        let (tx1_id_result, _) = &txs[0];
        let (tx2_id_result, _) = &txs[1];
        assert_eq!(*tx1_id_result, txid1);
        assert_eq!(*tx2_id_result, txid2);
    }

    #[tokio::test]
    async fn test_stats_updates() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        let initial_stats = state.stats();
        assert_eq!(initial_stats.mempool_size(), 0);
        assert_eq!(initial_stats.total_bytes(), 0);
        assert_eq!(initial_stats.enqueues_accepted(), 0);

        // Add first transaction - account 1 with seq_no 0
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1_size = ssz::Encode::as_ssz_bytes(&tx1).len();
        state.add_transaction(tx1.clone()).await.unwrap();

        let stats_after_first = state.stats();
        assert_eq!(stats_after_first.mempool_size(), 1);
        assert_eq!(stats_after_first.total_bytes(), tx1_size);
        assert_eq!(stats_after_first.enqueues_accepted(), 1);

        // Add second transaction - account 2 with seq_no 0
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        let tx2_size = ssz::Encode::as_ssz_bytes(&tx2).len();
        state.add_transaction(tx2).await.unwrap();

        let stats_after_second = state.stats();
        assert_eq!(stats_after_second.mempool_size(), 2);
        assert_eq!(stats_after_second.total_bytes(), tx1_size + tx2_size);
        assert_eq!(stats_after_second.enqueues_accepted(), 2);

        // Idempotent add (should not increment enqueues_accepted again)
        state.add_transaction(tx1).await.unwrap();

        let stats_after_idempotent = state.stats();
        assert_eq!(stats_after_idempotent.mempool_size(), 2);
        assert_eq!(stats_after_idempotent.enqueues_accepted(), 2);
    }

    #[tokio::test]
    async fn test_stats_rejections() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 2,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        let initial_stats = state.stats();
        assert_eq!(initial_stats.enqueues_rejected(), 0);
        assert_eq!(
            initial_stats
                .rejects_by_reason()
                .get(OLMempoolRejectReason::MempoolFull),
            0
        );
        assert_eq!(
            initial_stats
                .rejects_by_reason()
                .get(OLMempoolRejectReason::TransactionTooLarge),
            0
        );

        // Fill mempool to capacity - each to different account with seq_no 0
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        state.add_transaction(tx1).await.unwrap();
        state.add_transaction(tx2).await.unwrap();

        // Try to add when full
        let tx3 = create_test_snark_tx_with_seq_no(3, 0);
        let result = state.add_transaction(tx3).await;
        assert!(result.is_err());

        let stats_after_full = state.stats();
        assert_eq!(stats_after_full.enqueues_accepted(), 2);
        assert_eq!(stats_after_full.enqueues_rejected(), 1);
        assert_eq!(
            stats_after_full
                .rejects_by_reason()
                .get(OLMempoolRejectReason::MempoolFull),
            1
        );

        // Test transaction too large rejection
        let tip2 = create_test_block_commitment(100);
        let config_tiny = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 50,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider2 = Arc::new(create_test_state_provider(tip2));
        let context_tiny = Arc::new(create_test_context(config_tiny, provider2.clone()));
        let mut state2 = MempoolServiceState::new_with_context(context_tiny, tip2)
            .await
            .unwrap();

        let large_tx = create_test_tx_with_id(99);
        let result = state2.add_transaction(large_tx).await;
        assert!(result.is_err());

        let stats_after_large = state2.stats();
        assert_eq!(stats_after_large.enqueues_accepted(), 0);
        assert_eq!(stats_after_large.enqueues_rejected(), 1);
        assert_eq!(
            stats_after_large
                .rejects_by_reason()
                .get(OLMempoolRejectReason::TransactionTooLarge),
            1
        );
    }

    #[tokio::test]
    async fn test_remove_with_gap_cascade() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions for account 1: seq_no 0, 1, 2
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);

        let txid0 = state.add_transaction(tx0).await.unwrap();
        let txid1 = state.add_transaction(tx1).await.unwrap();
        let txid2 = state.add_transaction(tx2).await.unwrap();

        assert_eq!(state.stats().mempool_size(), 3);

        // Check account_state before removal: should have seq_nos [0, 1, 2]
        let account_id = create_test_account_id_with(1);
        let acct_state_before = state.account_state.get(&account_id).unwrap();
        assert_eq!(acct_state_before.seq_nos, BTreeSet::from([0, 1, 2]));
        assert_eq!(acct_state_before.seq_no_range(), Some((0, 2)));

        // Try adding seq_no=4 before removal - should fail with gap (2 -> 4, missing 3)
        let tx4_before = create_test_snark_tx_with_seq_no(1, 4);
        let result_gap = state.add_transaction(tx4_before).await;
        assert!(result_gap.is_err());
        assert!(matches!(
            result_gap.unwrap_err(),
            OLMempoolError::SequenceNumberGap { .. }
        ));

        // Remove middle transaction (seq_no 1) - creates gap!
        let removed = state.remove_transactions(&[txid1], true);

        // Should remove tx1 AND tx2 (cascade due to gap)
        assert_eq!(removed.len(), 2); // Both tx1 and tx2 removed
        assert!(removed.contains(&txid1));
        assert!(removed.contains(&txid2));

        // Only tx0 should remain
        assert_eq!(state.stats().mempool_size(), 1);
        assert!(state.contains(&txid0));
        assert!(!state.contains(&txid1));
        assert!(!state.contains(&txid2));

        // Check account_state after removal: should have only seq_no [0], so min=max=0
        let acct_state_after = state.account_state.get(&account_id).unwrap();
        assert_eq!(acct_state_after.seq_nos, BTreeSet::from([0]));
        assert_eq!(acct_state_after.seq_no_range(), Some((0, 0)));

        // Now try adding seq_no=4 again - should fail with gap (0 -> 4, missing 1,2,3)
        let tx4_after = create_test_snark_tx_with_seq_no(1, 4);
        let result4 = state.add_transaction(tx4_after).await;
        assert!(result4.is_err());
        assert!(matches!(
            result4.unwrap_err(),
            OLMempoolError::SequenceNumberGap { .. }
        ));

        // Adding seq_no=1 should work (no gap: 0 -> 1)
        let tx1_new = create_test_snark_tx_with_seq_no(1, 1);
        let result1 = state.add_transaction(tx1_new).await;
        assert!(result1.is_ok(), "Should accept seq_no 1 after gap removal");
    }

    #[tokio::test]
    async fn test_remove_transactions_no_cascade() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions for account 1: seq_no 0, 1, 2
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);

        let txid0 = state.add_transaction(tx0).await.unwrap();
        let txid1 = state.add_transaction(tx1).await.unwrap();
        let txid2 = state.add_transaction(tx2).await.unwrap();

        assert_eq!(state.stats().mempool_size(), 3);

        // Remove middle transaction WITHOUT cascade (simulating successful inclusion)
        let removed = state.remove_transactions(&[txid1], false);

        // Should only remove tx1 (no cascade)
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], txid1);

        // tx0 and tx2 should both remain
        assert_eq!(state.stats().mempool_size(), 2);
        assert!(state.contains(&txid0));
        assert!(!state.contains(&txid1));
        assert!(state.contains(&txid2));

        // Check account_state: should have seq_nos [0, 2] with a gap at 1
        let account = create_test_account_id_with(1);
        let acct_state = state.account_state.get(&account).unwrap();
        assert_eq!(acct_state.seq_nos, BTreeSet::from([0, 2]));
        assert_eq!(acct_state.seq_no_range(), Some((0, 2)));

        // Try adding seq_no=3 - should succeed (no gap: 2 -> 3)
        let tx3 = create_test_snark_tx_with_seq_no(1, 3);
        let result3 = state.add_transaction(tx3).await;
        assert!(result3.is_ok(), "Should accept seq_no 3 (no gap from 2)");

        // Verify seq_nos: should now have [0, 2, 3]
        let acct_state_after_3 = state.account_state.get(&account).unwrap();
        assert_eq!(acct_state_after_3.seq_nos, BTreeSet::from([0, 2, 3]));

        // Try adding seq_no=1 - should succeed (fills the gap)
        let tx1_new = create_test_snark_tx_with_seq_no(1, 1);
        let result1 = state.add_transaction(tx1_new).await;
        assert!(result1.is_ok(), "Should accept seq_no 1 (fills gap)");

        // Verify final seq_nos: should have [0, 1, 2, 3] - complete sequence
        let acct_state_final = state.account_state.get(&account).unwrap();
        assert_eq!(acct_state_final.seq_nos, BTreeSet::from([0, 1, 2, 3]));
        assert_eq!(acct_state_final.seq_no_range(), Some((0, 3)));
    }

    #[tokio::test]
    async fn test_revalidation_removes_expired_with_cascade() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions with seq_no 0, 1, 2 where middle one expires first
        let mut tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let mut tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let mut tx2 = create_test_snark_tx_with_seq_no(1, 2);

        // Set max_slot so tx1 expires at slot 110, others at 120
        tx0.attachment.max_slot = Optional::Some(120);
        tx1.attachment.max_slot = Optional::Some(110);
        tx2.attachment.max_slot = Optional::Some(120);

        let txid0 = state.add_transaction(tx0).await.unwrap();
        let txid1 = state.add_transaction(tx1).await.unwrap();
        let txid2 = state.add_transaction(tx2).await.unwrap();

        assert_eq!(state.stats().mempool_size(), 3);

        // Move to slot 111 where tx1 expires (max_slot=110, current=111 > 110)
        let tip_111 = create_test_block_commitment(111);
        provider.insert_state(tip_111, create_test_ol_state_for_tip(111));
        let state_accessor_111 = provider
            .get_state_for_tip_async(tip_111)
            .await
            .unwrap()
            .unwrap();
        state.state_accessor = state_accessor_111;

        // Revalidate all transactions - should find expired tx1
        let invalid_txids: Vec<OLTxId> = state.revalidate_all_transactions();
        assert_eq!(invalid_txids.len(), 1); // tx1 is invalid (expired)

        state.remove_transactions(&invalid_txids, true);

        // Should remove tx1 AND tx2 (cascade because tx1 expired creates gap)
        assert_eq!(state.stats().mempool_size(), 1);
        assert!(state.contains(&txid0));
        assert!(!state.contains(&txid1));
        assert!(!state.contains(&txid2));
    }

    #[tokio::test]
    async fn test_pending_seq_no_updated_both_methods() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        let account = create_test_account_id_with(1);

        // Add transactions: seq_no 0, 1, 2, 3
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);
        let tx3 = create_test_snark_tx_with_seq_no(1, 3);

        state.add_transaction(tx0).await.unwrap();
        let txid1 = state.add_transaction(tx1).await.unwrap();
        state.add_transaction(tx2).await.unwrap();
        state.add_transaction(tx3).await.unwrap();

        // pending_seq_no should be 3 (last seen)
        assert_eq!(
            state
                .account_state
                .get(&account)
                .and_then(|a| a.seq_nos.last().copied()),
            Some(3)
        );

        // Test remove_transactions (no cascade) - remove tx1
        state.remove_transactions(&[txid1], false);

        // pending_seq_no should still be 3 (max remaining is still 3)
        assert_eq!(
            state
                .account_state
                .get(&account)
                .and_then(|a| a.seq_nos.last().copied()),
            Some(3)
        );

        // Test remove_transactions_cascade - remove tx2 (should cascade to tx3)
        let tx2_id = state
            .entries
            .iter()
            .find(|(_, e)| {
                e.tx.base_update()
                    .is_some_and(|u| u.operation().seq_no() == 2)
            })
            .map(|(id, _)| *id)
            .unwrap();

        state.remove_transactions(&[tx2_id], true);

        // pending_seq_no should be 0 (last seen is now 0)
        assert_eq!(
            state
                .account_state
                .get(&account)
                .and_then(|a| a.seq_nos.last().copied()),
            Some(0)
        );
    }

    #[tokio::test]
    async fn test_transaction_size_limit() {
        let tip = create_test_block_commitment(100);
        let max_tx_size = 500; // Small size for testing
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Use a known account ID that exists in test state (accounts 0-255 are created)
        let account_200 = create_test_account_id_with(200);

        // Create a transaction that exceeds the size limit
        let oversized_tx = create_test_generic_tx_with_size(
            account_200,
            max_tx_size + 100,
            create_test_attachment_with_slots(None, None),
        );

        // Should be rejected due to size
        let result = state.add_transaction(oversized_tx).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::TransactionTooLarge { .. }
        ));

        // Verify stats tracked the rejection
        assert_eq!(state.stats().enqueues_rejected(), 1);
        assert_eq!(
            state
                .stats()
                .rejects_by_reason()
                .get(OLMempoolRejectReason::TransactionTooLarge),
            1
        );

        // Transaction within size limit should work
        let valid_tx = create_test_generic_tx_with_size(
            account_200,
            100, // Small payload that should fit
            create_test_attachment_with_slots(None, None),
        );

        let result = state.add_transaction(valid_tx).await;
        assert!(result.is_ok());
        assert_eq!(state.stats().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_transaction_replacement() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add first transaction with seq_no=0 for account 1
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let txid1 = state.add_transaction(tx1).await.unwrap();
        assert_eq!(state.stats().mempool_size(), 1);

        // Replace with new transaction with same seq_no=0
        let tx1_replacement = create_test_snark_tx_with_seq_no(1, 0);
        let txid1_replacement = state.add_transaction(tx1_replacement).await.unwrap();

        // Should have only 1 transaction (replacement replaced original)
        assert_eq!(state.stats().mempool_size(), 1);

        // Old transaction should be gone
        assert!(!state.contains(&txid1));

        // New transaction should be present
        assert!(state.contains(&txid1_replacement));
    }

    #[tokio::test]
    async fn test_transaction_duplicate_idempotent() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add first transaction with seq_no=0 for account 1
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let txid1 = state.add_transaction(tx1.clone()).await.unwrap();
        assert_eq!(state.stats().mempool_size(), 1);

        // Try to add exact same transaction again (idempotent)
        let txid1_again = state.add_transaction(tx1).await.unwrap();

        // Should return same txid
        assert_eq!(txid1, txid1_again);

        // Should still have only 1 transaction
        assert_eq!(state.stats().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_replacement_followed_by_sequential() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add first transaction with seq_no=0 for account 1
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        state.add_transaction(tx0).await.unwrap();

        // Replace with new transaction with same seq_no=0
        let tx0_replacement = create_test_snark_tx_with_seq_no(1, 0);
        state.add_transaction(tx0_replacement).await.unwrap();

        // Now add seq_no=1 (should work - sequential after replacement)
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let result = state.add_transaction(tx1).await;
        assert!(result.is_ok());

        // Should have 2 transactions (replaced tx0 + tx1)
        assert_eq!(state.stats().mempool_size(), 2);
    }

    #[tokio::test]
    async fn test_replacement_fails_with_invalid_slot_bounds() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions with seq_no 0, 1, 2
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);
        let txid0 = state.add_transaction(tx0).await.unwrap();
        let txid1 = state.add_transaction(tx1).await.unwrap();
        let txid2 = state.add_transaction(tx2).await.unwrap();
        assert_eq!(state.stats().mempool_size(), 3);

        // Try to replace seq_no=0 with transaction that has min_slot in the future (should fail)
        let tx0_invalid_future = create_test_snark_tx_with_seq_no_and_slots(1, 0, Some(200), None);
        let result_future = state.add_transaction(tx0_invalid_future).await;
        assert!(result_future.is_err());
        assert!(matches!(
            result_future.unwrap_err(),
            OLMempoolError::TransactionNotMature { .. }
        ));

        // Try to replace seq_no=0 with transaction that has max_slot passed (should fail)
        let tx0_invalid_expired = create_test_snark_tx_with_seq_no_and_slots(1, 0, None, Some(50));
        let result_expired = state.add_transaction(tx0_invalid_expired).await;
        assert!(result_expired.is_err());
        assert!(matches!(
            result_expired.unwrap_err(),
            OLMempoolError::TransactionExpired { .. }
        ));

        // All three original transactions should remain unaffected
        assert_eq!(state.stats().mempool_size(), 3);
        assert!(state.contains(&txid0));
        assert!(state.contains(&txid1));
        assert!(state.contains(&txid2));
    }

    #[tokio::test]
    async fn test_replay_attack_rejected() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions with seq_no 0, 1, 2
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);
        let txid0 = state.add_transaction(tx0).await.unwrap();
        state.add_transaction(tx1).await.unwrap();
        state.add_transaction(tx2).await.unwrap();

        // Now min_pending = 0, max_pending = 2
        // Remove tx0 from mempool to simulate it being mined, so min_pending = 1
        state.remove_transactions(&[txid0], false);

        // Now min_pending = 1, max_pending = 2
        // Try to add transaction with seq_no=0 (replay - less than min)
        let tx0_replay = create_test_snark_tx_with_seq_no(1, 0);
        let txid0_replay = tx0_replay.compute_txid();
        let result = state.add_transaction(tx0_replay).await;

        match result {
            Err(OLMempoolError::UsedSequenceNumber {
                txid,
                expected: account_seq_no,
                actual: tx_seq_no,
            }) => {
                assert_eq!(txid, txid0_replay);
                assert_eq!(tx_seq_no, 0);
                assert_eq!(account_seq_no, 1);
            }
            other => panic!("Expected InvalidSequenceNumber, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_replace_middle_transaction() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions with seq_no 0, 1, 2
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);
        let txid0 = state.add_transaction(tx0).await.unwrap();
        let txid1 = state.add_transaction(tx1).await.unwrap();
        let txid2 = state.add_transaction(tx2).await.unwrap();

        assert_eq!(state.stats().mempool_size(), 3);

        // Replace middle transaction (seq_no=1)
        let tx1_replacement = create_test_snark_tx_with_seq_no(1, 1);
        let txid1_replacement = state.add_transaction(tx1_replacement).await.unwrap();

        // Should still have 3 transactions
        assert_eq!(state.stats().mempool_size(), 3);

        // Original tx1 should be gone, replacement should be present
        assert!(!state.contains(&txid1));
        assert!(state.contains(&txid1_replacement));

        // tx0 and tx2 should still be present
        assert!(state.contains(&txid0));
        assert!(state.contains(&txid2));
    }

    #[tokio::test]
    async fn test_replace_min_transaction() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions with seq_no 0, 1, 2
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);
        let txid0 = state.add_transaction(tx0).await.unwrap();
        let txid1 = state.add_transaction(tx1).await.unwrap();
        let txid2 = state.add_transaction(tx2).await.unwrap();

        // Replace min transaction (seq_no=0)
        let tx0_replacement = create_test_snark_tx_with_seq_no(1, 0);
        let txid0_replacement = state.add_transaction(tx0_replacement).await.unwrap();

        assert_eq!(state.stats().mempool_size(), 3);
        assert!(!state.contains(&txid0));
        assert!(state.contains(&txid0_replacement));
        assert!(state.contains(&txid1));
        assert!(state.contains(&txid2));
    }

    #[tokio::test]
    async fn test_replace_max_transaction() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions with seq_no 0, 1, 2
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);
        let txid0 = state.add_transaction(tx0).await.unwrap();
        let txid1 = state.add_transaction(tx1).await.unwrap();
        let txid2 = state.add_transaction(tx2).await.unwrap();

        // Replace max transaction (seq_no=2)
        let tx2_replacement = create_test_snark_tx_with_seq_no(1, 2);
        let txid2_replacement = state.add_transaction(tx2_replacement).await.unwrap();

        assert_eq!(state.stats().mempool_size(), 3);
        assert!(state.contains(&txid0));
        assert!(state.contains(&txid1));
        assert!(!state.contains(&txid2));
        assert!(state.contains(&txid2_replacement));
    }

    #[tokio::test]
    async fn test_multiple_replacements_same_tx() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add initial transaction
        let tx0_v1 = create_test_snark_tx_with_seq_no(1, 0);
        let txid0_v1 = state.add_transaction(tx0_v1).await.unwrap();

        // Replace once
        let tx0_v2 = create_test_snark_tx_with_seq_no(1, 0);
        let txid0_v2 = state.add_transaction(tx0_v2).await.unwrap();

        // Replace again
        let tx0_v3 = create_test_snark_tx_with_seq_no(1, 0);
        let txid0_v3 = state.add_transaction(tx0_v3).await.unwrap();

        // Should have only 1 transaction (latest version)
        assert_eq!(state.stats().mempool_size(), 1);
        assert!(!state.contains(&txid0_v1));
        assert!(!state.contains(&txid0_v2));
        assert!(state.contains(&txid0_v3));
    }

    #[tokio::test]
    async fn test_replacement_updates_pending_seq_no() {
        let tip = create_test_block_commitment(100);
        let config = OLMempoolConfig {
            max_tx_count: 10,
            max_tx_size: 1_000_000,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        };
        let provider = Arc::new(create_test_state_provider(tip));
        let context = Arc::new(create_test_context(config, provider.clone()));
        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();

        // Add transactions with seq_no 0, 1, 2
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);
        state.add_transaction(tx0).await.unwrap();
        state.add_transaction(tx1).await.unwrap();
        state.add_transaction(tx2).await.unwrap();

        // pending_seq_no should be 2 (max)
        let account_id = create_test_account_id_with(1);
        assert_eq!(
            state
                .account_state
                .get(&account_id)
                .and_then(|a| a.seq_nos.last().copied()),
            Some(2)
        );

        // Replace middle transaction (seq_no=1)
        let tx1_replacement = create_test_snark_tx_with_seq_no(1, 1);
        state.add_transaction(tx1_replacement).await.unwrap();

        // pending_seq_no should still be 2 (max unchanged)
        assert_eq!(
            state
                .account_state
                .get(&account_id)
                .and_then(|a| a.seq_nos.last().copied()),
            Some(2)
        );

        // Add sequential transaction (seq_no=3)
        let tx3 = create_test_snark_tx_with_seq_no(1, 3);
        state.add_transaction(tx3).await.unwrap();

        // pending_seq_no should now be 3
        assert_eq!(
            state
                .account_state
                .get(&account_id)
                .and_then(|a| a.seq_nos.last().copied()),
            Some(3)
        );
    }
}
