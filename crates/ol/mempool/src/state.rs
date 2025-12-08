//! Mempool service state management.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use strata_acct_types::AccountId;
use strata_db_types::types::MempoolTxData;
use strata_identifiers::{OLBlockCommitment, OLTxId};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_ol_chain_types_new::{OLBlock, OLTransaction, TransactionPayload};
use strata_ol_state_types::OLState;
use strata_service::ServiceState;
use strata_storage::NodeStorage;

use crate::{
    OLMempoolError, OLMempoolResult,
    ordering::{FifoOrderingStrategy, OrderingStrategy},
    types::{
        MempoolEntry, MempoolOrderingKey, OLMempoolConfig, OLMempoolRejectReason, OLMempoolStats,
        OLMempoolTransaction,
    },
    validation::{BasicTransactionValidator, TransactionValidator},
};

/// Immutable context for mempool service (shared via Arc).
/// Used in tests and will be used in service implementation.
pub(crate) struct MempoolContext {
    /// Mempool configuration.
    pub(crate) config: OLMempoolConfig,

    /// Storage backend for creating StateAccessor instances.
    /// Mempool creates StateAccessor per-operation based on current_tip.
    pub(crate) storage: Arc<NodeStorage>,

    /// Ordering strategy for transaction priority.
    pub(crate) ordering_strategy: Arc<dyn OrderingStrategy>,

    /// Transaction validator for validation strategies.
    /// Uses concrete type since TransactionValidator trait is not object-safe (has generic
    /// method).
    pub(crate) validator: Arc<dyn TransactionValidator>,
}

/// Mutable state for mempool service (owned by service task).
pub(crate) struct MempoolState {
    /// In-memory entries indexed by transaction ID.
    entries: HashMap<OLTxId, MempoolEntry>,

    /// Ordering index: priority → transaction ID.
    /// BTreeMap provides ordered iteration by priority (lower = higher priority).
    /// Priority is u128 encoding (primary_key << 64) | insertion_id.
    ordering_index: BTreeMap<u128, OLTxId>,

    /// Next insertion ID for deterministic ordering.
    next_insertion_id: u64,

    /// Current chain tip (block ID + slot).
    current_tip: OLBlockCommitment,

    /// Track expected sequence number per account (for gap detection).
    /// For each account, stores the next expected seq_no (current + pending count).
    pending_seq_no: HashMap<AccountId, u64>,

    /// Cached StateAccessor for the current tip.
    /// StateAccessor is created when tip changes in `set_current_tip()`.
    state_accessor: Option<Arc<OLState>>,

    /// Mempool statistics.
    stats: OLMempoolStats,
}

impl MempoolState {
    /// Create new mempool state.
    ///
    /// State accessor will be fetched lazily when needed via get_or_fetch_state_accessor.
    /// This avoids blocking calls in async contexts.
    pub(crate) fn new(current_tip: OLBlockCommitment) -> Self {
        let state_accessor = None;

        Self {
            entries: HashMap::new(),
            ordering_index: BTreeMap::new(),
            next_insertion_id: 0,
            current_tip,
            pending_seq_no: HashMap::new(),
            state_accessor,
            stats: OLMempoolStats::default(),
        }
    }

    /// Count pending transactions for an account.
    ///
    /// Returns the number of pending transactions (both SnarkAccountUpdate and
    /// GenericAccountMessage) for the given account.
    ///
    /// Note: For seq_no validation, only SnarkAccountUpdate transactions should be counted.
    /// The `pending_seq_no` HashMap already tracks the next expected seq_no per account.
    #[expect(dead_code, reason = "will be used in validation and revalidation")]
    pub(crate) fn count_pending_for_account(&self, account: AccountId) -> u64 {
        self.entries
            .values()
            .filter(|entry| entry.tx.target() == account)
            .count() as u64
    }

    /// Add a transaction to the mempool.
    ///
    /// Returns the transaction ID. Idempotent - returns existing txid if duplicate.
    pub(crate) async fn add_transaction(
        &mut self,
        ctx: &MempoolContext,
        tx: OLMempoolTransaction,
    ) -> OLMempoolResult<OLTxId> {
        let txid = tx.compute_txid();

        // Idempotent check - if already present, return success
        if self.entries.contains_key(&txid) {
            return Ok(txid);
        }

        // Validate capacity limits
        if self.entries.len() >= ctx.config.max_tx_count {
            self.stats.enqueues_rejected += 1;
            self.stats
                .rejects_by_reason
                .increment(OLMempoolRejectReason::MempoolFull);
            return Err(OLMempoolError::MempoolFull {
                current: self.entries.len(),
                limit: ctx.config.max_tx_count,
            });
        }

        // Compute transaction size
        let tx_size = ssz::Encode::as_ssz_bytes(&tx).len();

        if tx_size > ctx.config.max_tx_size {
            self.stats.enqueues_rejected += 1;
            self.stats
                .rejects_by_reason
                .increment(OLMempoolRejectReason::TransactionTooLarge);
            return Err(OLMempoolError::TransactionTooLarge {
                size: tx_size,
                limit: ctx.config.max_tx_size,
            });
        }

        // Get or create StateAccessor for current tip
        let state_accessor = self.ensure_state_accessor(ctx).await?;

        // Validate transaction using state accessor
        ctx.validator
            .validate(&tx, &self.current_tip, &state_accessor)?;

        // Gap checking for SnarkAccountUpdate transactions
        // We reject transactions that would create sequence number gaps
        if let Some(base_update) = tx.base_update() {
            let target_account = tx.target();
            let tx_seq_no = base_update.operation().seq_no();

            // Get current on-chain sequence number for this account
            let account_state = state_accessor
                .get_account_state(target_account)
                .map_err(|e| OLMempoolError::AccountStateAccess(e.to_string()))?
                .ok_or(OLMempoolError::AccountDoesNotExist {
                    account: target_account,
                })?;

            let current_on_chain_seq_no = match account_state.as_snark_account() {
                Ok(snark_state) => *snark_state.seqno().inner(),
                Err(_) => {
                    // Not a snark account - this should have been caught by validator
                    return Err(OLMempoolError::AccountTypeMismatch {
                        txid,
                        account: target_account,
                    });
                }
            };

            // Get expected sequence number from pending_seq_no HashMap (O(1) lookup)
            // If not tracked yet, expected is current on-chain seq_no
            let expected_seq_no = self
                .pending_seq_no
                .get(&target_account)
                .copied()
                .unwrap_or(current_on_chain_seq_no);

            // Reject if there's a gap
            if tx_seq_no != expected_seq_no {
                return Err(OLMempoolError::SequenceNumberGap {
                    expected: expected_seq_no,
                    actual: tx_seq_no,
                });
            }

            // Update tracking: next expected is tx_seq_no + 1
            self.pending_seq_no.insert(target_account, tx_seq_no + 1);
        }

        // Get insertion_id for ordering
        let insertion_id = self.next_insertion_id;
        self.next_insertion_id += 1;

        // Create entry
        let ordering_key = MempoolOrderingKey::new(self.current_tip.slot(), insertion_id);
        let entry = MempoolEntry::new(tx.clone(), ordering_key, tx_size);

        // Compute priority using strategy (assumed unique)
        let priority = ctx.ordering_strategy.compute_priority(&entry);

        // Persist to database FIRST (ensures durability before in-memory operations)
        let tx_data = MempoolTxData::new(
            txid,
            ssz::Encode::as_ssz_bytes(&entry.tx),
            self.current_tip.slot(),
            insertion_id,
        );
        ctx.storage.mempool().put_tx(tx_data)?;

        // Add to ordering index
        self.ordering_index.insert(priority, txid);

        // Add to entries
        self.entries.insert(txid, entry);

        // Update stats
        self.stats.mempool_size += 1;
        self.stats.total_bytes += tx_size;
        self.stats.enqueues_accepted += 1;

        Ok(txid)
    }

    /// Get up to `limit` transactions in priority order.
    ///
    /// Returns (txid, transaction) pairs.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "used in tests and will be used in service implementation"
        )
    )]
    pub(crate) fn get_transactions(
        &self,
        limit: usize,
    ) -> OLMempoolResult<Vec<(OLTxId, OLMempoolTransaction)>> {
        let mut result = Vec::with_capacity(limit.min(self.entries.len()));

        // Iterate ordering index (BTreeMap sorted by priority)
        for txid in self.ordering_index.values() {
            if result.len() >= limit {
                return Ok(result);
            }

            if let Some(entry) = self.entries.get(txid) {
                result.push((*txid, entry.tx.clone()));
            }
        }

        Ok(result)
    }

    /// Remove transactions from the mempool.
    ///
    /// Returns the removed transaction IDs.
    pub(crate) fn remove_transactions(
        &mut self,
        ctx: &MempoolContext,
        ids: &[OLTxId],
    ) -> OLMempoolResult<Vec<OLTxId>> {
        let mut removed = Vec::with_capacity(ids.len());
        let mut account_min_seq: HashMap<AccountId, u64> = HashMap::new();

        // First pass: Remove specified transactions and collect minimum seq_no per account
        for txid in ids {
            // Check if entry exists and get necessary data before removing
            if let Some(entry) = self.entries.get(txid) {
                // Remove from database FIRST (ensures durability - if this fails, memory unchanged)
                ctx.storage.mempool().del_tx(*txid)?;

                // Recompute priority for ordering index removal
                let priority = ctx.ordering_strategy.compute_priority(entry);
                let size_bytes = entry.size_bytes;

                // Track minimum seq_no for each affected account (for cascade removal)
                let account_info = if let Some(base_update) = entry.tx.base_update() {
                    let account = entry.tx.target();
                    let seq_no = base_update.operation().seq_no();
                    Some((account, seq_no))
                } else {
                    None
                };

                // Now remove from memory (safe because DB operation succeeded)
                self.entries.remove(txid);
                self.ordering_index.remove(&priority);

                // Update account tracking
                if let Some((account, seq_no)) = account_info {
                    account_min_seq
                        .entry(account)
                        .and_modify(|min_seq| *min_seq = (*min_seq).min(seq_no))
                        .or_insert(seq_no);
                }

                // Update stats
                self.stats.mempool_size -= 1;
                self.stats.total_bytes -= size_bytes;

                removed.push(*txid);
            }
        }

        // Second pass: Cascade-remove dependent transactions and recalculate pending_seq_no
        for (account, min_failed_seq) in account_min_seq {
            self.cascade_remove_for_account(ctx, account, min_failed_seq, &mut removed)?;
        }

        Ok(removed)
    }

    /// Helper: Cascade-remove transactions for an account starting from minimum failed seq_no.
    ///
    /// Removes all transactions with seq_no >= min_failed_seq, then recalculates pending_seq_no.
    fn cascade_remove_for_account(
        &mut self,
        ctx: &MempoolContext,
        account: AccountId,
        min_failed_seq: u64,
        removed: &mut Vec<OLTxId>,
    ) -> OLMempoolResult<()> {
        let mut max_remaining_seq: Option<u64> = None;
        let mut to_remove = Vec::new();

        // Collect txids to remove and track max remaining seq_no
        for (txid, entry) in &self.entries {
            if entry.tx.target() != account {
                continue;
            }

            if let Some(base_update) = entry.tx.base_update() {
                let seq_no = base_update.operation().seq_no();
                if seq_no >= min_failed_seq {
                    to_remove.push(*txid);
                } else {
                    max_remaining_seq =
                        Some(max_remaining_seq.map_or(seq_no, |max| max.max(seq_no)));
                }
            }
        }

        // Remove and add to removed list
        for txid in to_remove {
            // Check if entry exists and get necessary data before removing
            if let Some(entry) = self.entries.get(&txid) {
                // Remove from database FIRST (ensures durability - if this fails, memory unchanged)
                ctx.storage.mempool().del_tx(txid)?;

                // Compute priority for ordering index removal
                let priority = ctx.ordering_strategy.compute_priority(entry);
                let size_bytes = entry.size_bytes;

                // Now remove from memory (safe because DB operation succeeded)
                self.entries.remove(&txid);
                self.ordering_index.remove(&priority);

                // Update stats
                self.stats.mempool_size -= 1;
                self.stats.total_bytes -= size_bytes;

                removed.push(txid);
            }
        }

        // Update pending_seq_no
        if let Some(max_seq) = max_remaining_seq {
            self.pending_seq_no.insert(account, max_seq + 1);
        } else {
            self.pending_seq_no.remove(&account);
        }

        Ok(())
    }

    /// Check if a transaction exists in the mempool.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "used in tests and will be used in service implementation"
        )
    )]
    pub(crate) fn contains(&self, id: &OLTxId) -> bool {
        self.entries.contains_key(id)
    }

    /// Get mempool statistics.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "used in tests and will be used in service implementation"
        )
    )]
    pub(crate) fn stats(&self) -> OLMempoolStats {
        self.stats.clone()
    }

    /// Update the current chain tip and clear cached state accessor.
    ///
    /// When the tip changes, the cached state accessor is invalidated and will be
    /// fetched lazily when needed via `ensure_state_accessor()`.
    pub(crate) fn set_current_tip(&mut self, tip: OLBlockCommitment) {
        if self.current_tip != tip {
            self.current_tip = tip;
            // Clear cached state accessor since tip changed (old state is invalid)
            self.state_accessor = None;
        }
    }

    /// Get current chain tip.
    #[expect(dead_code, reason = "will be used in service implementation")]
    pub(crate) fn current_tip(&self) -> OLBlockCommitment {
        self.current_tip
    }

    /// Ensures state accessor is available for the current tip, fetching and caching it if needed.
    ///
    /// This method is used by both `set_current_tip` (when we want to eagerly fetch)
    /// and other methods that need the state accessor. It caches the result to avoid
    /// repeated database lookups.
    async fn ensure_state_accessor(
        &mut self,
        ctx: &MempoolContext,
    ) -> OLMempoolResult<Arc<OLState>> {
        // Return cached accessor if available
        if let Some(accessor) = &self.state_accessor {
            return Ok(accessor.clone());
        }

        // Fetch state accessor for current tip (async)
        let accessor = ctx
            .storage
            .ol_state()
            .get_toplevel_ol_state_async(self.current_tip)
            .await
            .map_err(|e| {
                OLMempoolError::AccountStateAccess(format!(
                    "Failed to get state for tip {:?}: {e}",
                    self.current_tip
                ))
            })?
            .ok_or_else(|| {
                OLMempoolError::AccountStateAccess(format!(
                    "State not found for tip {:?}",
                    self.current_tip
                ))
            })?;

        // Cache it for future use (accessor is already Arc<OLState>)
        self.state_accessor = Some(accessor.clone());
        Ok(accessor)
    }

    /// Gets the parent commitment of a given block commitment.
    ///
    /// Returns None if the block doesn't exist, is at slot 0 (genesis has no parent), or on error.
    async fn get_parent_commitment(
        ctx: &MempoolContext,
        commitment: OLBlockCommitment,
    ) -> Option<OLBlockCommitment> {
        let current_slot = commitment.slot();
        // Genesis block (slot 0) has no parent
        if current_slot == 0 {
            return None;
        }

        let block = ctx
            .storage
            .ol_block()
            .get_block_data_async(&commitment)
            .await
            .ok()??;
        let parent_blkid = *block.header().parent_blkid();
        let parent_slot = current_slot - 1;
        Some(OLBlockCommitment::new(parent_slot, parent_blkid))
    }

    /// Finds the common ancestor (pivot) by walking backwards from both tips.
    ///
    /// Returns the pivot block commitment where the two chains meet.
    async fn find_common_ancestor(
        ctx: &MempoolContext,
        old_tip: OLBlockCommitment,
        new_tip: OLBlockCommitment,
    ) -> OLMempoolResult<OLBlockCommitment> {
        // Build a HashSet of ancestors from old_tip for O(1) lookup
        let mut old_ancestors = HashSet::new();
        let mut current = old_tip;
        let max_depth = ctx.config.max_reorg_depth;

        // Walk backwards from old_tip, storing all ancestors
        for _ in 0..max_depth {
            old_ancestors.insert(current);
            match Self::get_parent_commitment(ctx, current).await {
                Some(parent) => current = parent,
                None => break,
            }
        }

        // Walk backwards from new_tip, checking for intersection with old_ancestors
        current = new_tip;
        for _ in 0..max_depth {
            // Check if we've found a common ancestor
            if old_ancestors.contains(&current) {
                return Ok(current);
            }
            match Self::get_parent_commitment(ctx, current).await {
                Some(parent) => current = parent,
                None => break,
            }
        }

        // If we couldn't find a common ancestor, return the one with the lower slot
        // (this shouldn't happen in practice, but provides a fallback)
        if old_tip.slot() < new_tip.slot() {
            Ok(old_tip)
        } else {
            Ok(new_tip)
        }
    }

    /// Extracts transactions from a block, converting them to mempool transactions.
    ///
    /// Returns a vector of converted mempool transactions (without accumulator proofs).
    /// Transactions that fail to convert are skipped.
    fn extract_transactions_from_block(block: &OLBlock) -> Vec<OLMempoolTransaction> {
        let mut transactions = Vec::new();
        if let Some(tx_segment) = block.body().tx_segment() {
            for tx in tx_segment.txs() {
                // Convert to mempool transaction (removes accumulator proofs)
                if let Ok(mempool_tx) = Self::convert_block_tx_to_mempool_tx(tx) {
                    transactions.push(mempool_tx);
                }
            }
        }
        transactions
    }

    /// Extracts transaction IDs from a block.
    ///
    /// Converts block transactions to mempool format and computes their txids.
    /// This ensures the txid matches what's stored in the mempool (without accumulator proofs).
    fn extract_txids_from_block(block: &OLBlock) -> Vec<OLTxId> {
        Self::extract_transactions_from_block(block)
            .iter()
            .map(|tx| tx.compute_txid())
            .collect()
    }

    /// Converts a block transaction to a mempool transaction by removing accumulator proofs.
    ///
    /// For SnarkAccountUpdate transactions, this extracts only the base_update without
    /// accumulator_proofs. For GenericAccountMessage transactions, this is a direct conversion.
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

    /// Revalidates all transactions and returns IDs of invalid ones.
    fn revalidate_all_transactions(
        &self,
        ctx: &MempoolContext,
        state_accessor: &OLState,
    ) -> Vec<OLTxId> {
        let mut invalid_txids = Vec::new();
        for (txid, entry) in &self.entries {
            if ctx
                .validator
                .validate(&entry.tx, &self.current_tip, state_accessor)
                .is_err()
            {
                invalid_txids.push(*txid);
            }
        }
        invalid_txids
    }

    /// Handles a reorg: revalidates existing transactions, removes those with gaps,
    /// and adds transactions from rolled-back blocks back to the mempool.
    ///
    /// This method:
    /// 1. Revalidates existing transactions and removes invalid ones
    /// 2. Verifies sequential seq_no ordering per account and removes transactions with gaps
    /// 3. Finds all blocks that were rolled back (between old_tip and new_tip)
    /// 4. Extracts transactions from those rolled-back blocks
    /// 5. Converts them to mempool transactions (removing accumulator proofs)
    /// 6. Adds them back to the mempool (they will be validated during add)
    /// 7. Recalculates pending_seq_no tracking
    async fn handle_reorg(
        &mut self,
        ctx: &MempoolContext,
        old_tip: OLBlockCommitment,
        new_tip: OLBlockCommitment,
        state_accessor: &OLState,
    ) -> OLMempoolResult<usize> {
        // Step 1: Revalidate existing transactions and remove invalid ones
        let invalid_txids = self.revalidate_all_transactions(ctx, state_accessor);
        let mut removed_count = invalid_txids.len();
        if !invalid_txids.is_empty() {
            self.remove_transactions(ctx, &invalid_txids)?;
        }

        // Step 2: Verify sequential seq_no ordering and remove transactions with gaps
        // For each account, collect SnarkAccountUpdate transactions and verify sequential ordering
        let mut txs_by_account: HashMap<AccountId, Vec<(u64, OLTxId)>> = HashMap::new();
        for (txid, entry) in &self.entries {
            if let Some(base_update) = entry.tx.base_update() {
                let target_account = entry.tx.target();
                let seq_no = base_update.operation().seq_no();
                txs_by_account
                    .entry(target_account)
                    .or_default()
                    .push((seq_no, *txid));
            }
        }

        // For each account, sort by seq_no and remove transactions with gaps
        let mut gap_txids = Vec::new();
        for (account, mut txs) in txs_by_account {
            // Sort by seq_no
            txs.sort_by_key(|(seq_no, _)| *seq_no);

            // Get on-chain seq_no for this account
            let on_chain_seq_no = state_accessor
                .get_account_state(account)
                .map_err(|e| {
                    OLMempoolError::AccountStateAccess(format!(
                        "Failed to get account state during reorg: {e}"
                    ))
                })?
                .and_then(|account_state| {
                    // Try to get as snark account, return 0 if not snark or error
                    account_state
                        .as_snark_account()
                        .map(|snark_state| *snark_state.seqno().inner())
                        .ok()
                })
                .unwrap_or(0);

            // Verify sequential ordering starting from on_chain_seq_no
            let mut expected_seq_no = on_chain_seq_no;
            for (seq_no, txid) in txs {
                if seq_no != expected_seq_no {
                    // Gap detected: this transaction and all subsequent ones from this account are
                    // invalid
                    gap_txids.push(txid);
                } else {
                    // Sequential - move to next expected seq_no
                    expected_seq_no += 1;
                }
            }
        }

        // Remove transactions with gaps
        if !gap_txids.is_empty() {
            removed_count += self.remove_transactions(ctx, &gap_txids)?.len();
        }

        // Step 3: Find the common ancestor (pivot) by walking backwards from both tips
        let pivot = Self::find_common_ancestor(ctx, old_tip, new_tip).await?;

        // Step 4: Collect all blocks from old_tip down to (but not including) the pivot
        // These are blocks that were rolled back
        let mut rolled_back_blocks = Vec::new();
        let mut current_commitment = old_tip;

        while current_commitment != pivot {
            // Get the block to add to rolled-back list
            let block = match ctx
                .storage
                .ol_block()
                .get_block_data_async(&current_commitment)
                .await
            {
                Ok(Some(block)) => block,
                Ok(None) => {
                    // Block not found, stop searching
                    break;
                }
                Err(_e) => {
                    // Block not found or error accessing it, stop searching
                    break;
                }
            };

            // Add this block to rolled-back list
            rolled_back_blocks.push(block);

            // Get parent commitment using helper
            match Self::get_parent_commitment(ctx, current_commitment).await {
                Some(parent) => current_commitment = parent,
                None => break,
            }
        }

        // Step 5: Collect all blocks from pivot to new_tip (the new chain)
        // These are blocks whose transactions need to be removed from the mempool
        let mut new_chain_blocks = Vec::new();
        current_commitment = new_tip;

        while current_commitment != pivot {
            // Get the block
            let block = match ctx
                .storage
                .ol_block()
                .get_block_data_async(&current_commitment)
                .await
            {
                Ok(Some(block)) => block,
                Ok(None) => {
                    // Block not found, stop searching
                    break;
                }
                Err(_e) => {
                    // Block not found or error accessing it, stop searching
                    break;
                }
            };

            // Add this block to new chain list
            new_chain_blocks.push(block);

            // Get parent commitment using helper
            match Self::get_parent_commitment(ctx, current_commitment).await {
                Some(parent) => current_commitment = parent,
                None => break,
            }
        }

        // Step 6: Remove transactions from new chain blocks (they're now in blocks)
        let mut new_chain_txids = HashSet::new();
        for block in &new_chain_blocks {
            let txids = Self::extract_txids_from_block(block);
            new_chain_txids.extend(txids);
        }
        if !new_chain_txids.is_empty() {
            let txids_vec: Vec<_> = new_chain_txids.iter().copied().collect();
            removed_count += self.remove_transactions(ctx, &txids_vec)?.len();
        }

        // Step 7: Extract transactions from rolled-back blocks that aren't in the new chain
        // and add them back to mempool
        for block in rolled_back_blocks.iter().rev() {
            // Process blocks in reverse order (oldest first) to maintain transaction ordering
            let mempool_txs = Self::extract_transactions_from_block(block);
            for mempool_tx in mempool_txs {
                let txid = mempool_tx.compute_txid();
                // Only re-add if not already in the new chain
                if !new_chain_txids.contains(&txid) {
                    // Add transaction back to mempool (will be validated)
                    // If add_transaction fails (e.g., duplicate, invalid), we just skip it
                    let _ = self.add_transaction(ctx, mempool_tx).await;
                }
            }
        }

        // Step 8: Recalculate pending_seq_no tracking from current mempool transactions
        // (including newly added ones from rolled-back blocks)
        // With sequential enforcement, we can simply rebuild by iterating in seq_no order
        self.pending_seq_no.clear();
        for entry in self.entries.values() {
            if let Some(base_update) = entry.tx.base_update() {
                let target_account = entry.tx.target();
                let tx_seq_no = base_update.operation().seq_no();
                let next_expected = tx_seq_no + 1;
                // Sequential enforcement: just set next_expected
                self.pending_seq_no.insert(target_account, next_expected);
            }
        }

        Ok(removed_count)
    }

    /// Handles a new block: removes included transactions and revalidates remaining ones.
    ///
    /// This method:
    /// 1. Fetches the new block from OL block database
    /// 2. Extracts transaction IDs from the block
    /// 3. Removes those transactions from the mempool (they're now in a block)
    /// 4. Revalidates remaining transactions (state may have changed)
    async fn handle_new_block(
        &mut self,
        ctx: &MempoolContext,
        new_tip: OLBlockCommitment,
        state_accessor: &OLState,
    ) -> OLMempoolResult<usize> {
        // Step 1: Fetch new block from OL block database
        let block = ctx
            .storage
            .ol_block()
            .get_block_data_async(&new_tip)
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

        // Step 2: Extract transaction IDs from block
        let included_txids = Self::extract_txids_from_block(&block);

        // Step 3: Remove included transactions from mempool
        let mut removed_count = 0;
        if !included_txids.is_empty() {
            removed_count = self.remove_transactions(ctx, &included_txids)?.len();
        }

        // Step 4: Revalidate remaining transactions (state may have changed)
        let invalid_txids = self.revalidate_all_transactions(ctx, state_accessor);
        if !invalid_txids.is_empty() {
            removed_count += self.remove_transactions(ctx, &invalid_txids)?.len();
        }

        Ok(removed_count)
    }

    /// Handle chain update: update tip and revalidate all transactions.
    ///
    /// This method:
    /// 1. Updates the current tip and state accessor
    /// 2. Revalidates all pending transactions against the new state
    /// 3. Removes invalid transactions
    /// 4. Returns the count of removed transactions
    #[expect(
        dead_code,
        reason = "used in tests and will be used in service implementation"
    )]
    pub(crate) async fn handle_chain_update(
        &mut self,
        ctx: &MempoolContext,
        new_tip: OLBlockCommitment,
    ) -> OLMempoolResult<usize> {
        // Check if this is a reorg (rollback) or forward progress (new block)
        // A reorg occurs if:
        // 1. New tip is at a lower slot (rollback), OR
        // 2. New tip is at the same slot but different block ID (same-slot reorg), OR
        // 3. New tip is at a higher slot but not a descendant of current tip (different fork)
        // Otherwise, it's forward progress (new block)
        let is_reorg = if new_tip.slot() < self.current_tip.slot() {
            // Lower slot = definitely a rollback
            true
        } else if new_tip.slot() == self.current_tip.slot() {
            // Same slot but different block ID = same-slot reorg
            new_tip != self.current_tip
        } else {
            // Higher slot: check if current_tip is an ancestor of new_tip
            // Walk backwards from new_tip until we either find current_tip (new block)
            // or reach a slot <= current_tip.slot() without finding it (reorg)
            let mut current = new_tip;
            let max_walk = (new_tip.slot() - self.current_tip.slot()).min(10); // Limit walk to 10 blocks
            let mut found_current_tip = false;

            for _ in 0..max_walk {
                if current == self.current_tip {
                    // Found current_tip as ancestor, so it's a new block
                    found_current_tip = true;
                    break;
                }
                if current.slot() <= self.current_tip.slot() {
                    // Reached current_tip's slot or below without finding it, so it's a reorg
                    break;
                }
                match Self::get_parent_commitment(ctx, current).await {
                    Some(parent) => current = parent,
                    None => {
                        // Can't walk further, treat as reorg to be safe
                        break;
                    }
                }
            }
            // If we haven't found current_tip, it's a reorg (different fork)
            !found_current_tip
        };
        let old_tip = self.current_tip;

        // Update tip and clear cached state accessor
        self.set_current_tip(new_tip);

        // Ensure state accessor is available
        let state_accessor = self.ensure_state_accessor(ctx).await?;

        // Handle based on whether it's a reorg or new block
        if is_reorg {
            self.handle_reorg(ctx, old_tip, new_tip, &state_accessor)
                .await
        } else {
            self.handle_new_block(ctx, new_tip, &state_accessor).await
        }
    }

    /// Load all transactions from database on startup.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "used in tests and will be used in service implementation"
        )
    )]
    pub(crate) fn load_from_db(&mut self, ctx: &MempoolContext) -> OLMempoolResult<()> {
        let all_txs = ctx.storage.mempool().get_all_txs()?;

        // Track max insertion_id to continue from
        let mut max_insertion_id: u64 = 0;

        for tx_data in all_txs {
            // Parse transaction from bytes
            let tx = ssz::Decode::from_ssz_bytes(&tx_data.tx_bytes).map_err(|e| {
                OLMempoolError::Serialization(format!(
                    "Failed to decode transaction {}: {:?}",
                    tx_data.txid, e
                ))
            })?;

            // Track max insertion_id
            max_insertion_id = max_insertion_id.max(tx_data.insertion_id);

            // Create ordering key from persisted data
            let ordering_key =
                MempoolOrderingKey::new(tx_data.first_seen_slot, tx_data.insertion_id);

            // Create entry
            let entry = MempoolEntry::new(tx, ordering_key, tx_data.tx_bytes.len());

            // Compute priority using strategy
            let priority = ctx.ordering_strategy.compute_priority(&entry);

            // Add to in-memory indices
            self.ordering_index.insert(priority, tx_data.txid);
            self.entries.insert(tx_data.txid, entry);
        }

        // Set next_insertion_id to continue from max + 1
        self.next_insertion_id = max_insertion_id.saturating_add(1);

        // Recompute stats
        self.stats.mempool_size = self.entries.len();
        self.stats.total_bytes = self.entries.values().map(|e| e.size_bytes).sum();

        Ok(())
    }
}

/// Combined state for the service (context + mutable state).
#[expect(dead_code, reason = "will be used in service implementation")]
pub(crate) struct MempoolServiceState {
    pub(crate) ctx: Arc<MempoolContext>,
    pub(crate) state: MempoolState,
}

impl MempoolServiceState {
    /// Create new mempool service state.
    #[expect(dead_code, reason = "will be used in service implementation")]
    pub(crate) fn new(ctx: Arc<MempoolContext>, current_tip: OLBlockCommitment) -> Self {
        Self {
            ctx: ctx.clone(),
            state: MempoolState::new(current_tip),
        }
    }
}

impl MempoolContext {
    /// Create a new mempool context with FIFO ordering strategy.
    #[expect(
        dead_code,
        reason = "used in tests and will be used in service implementation"
    )]
    pub(crate) fn new(config: OLMempoolConfig, storage: Arc<NodeStorage>) -> Self {
        let validator_config = config.clone();
        Self {
            config,
            storage,
            ordering_strategy: Arc::new(FifoOrderingStrategy),
            validator: Arc::new(BasicTransactionValidator::new(validator_config)),
        }
    }
}

impl ServiceState for MempoolServiceState {
    fn name(&self) -> &str {
        "mempool"
    }
}

#[cfg(test)]
mod tests {
    use strata_identifiers::Buf32;

    use super::*;
    use crate::test_utils::{
        create_test_block_commitment, create_test_context, create_test_context_with_state,
        create_test_snark_tx_with_seq_no, create_test_tx_with_id,
    };

    #[tokio::test]
    async fn test_add_transaction() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let txid1 = state.add_transaction(&ctx, tx1.clone()).await.unwrap();

        // Transaction should be in mempool
        assert!(state.contains(&txid1));
        assert_eq!(state.stats().mempool_size(), 1);

        // Idempotent - adding again should succeed
        let txid1_again = state.add_transaction(&ctx, tx1).await.unwrap();
        assert_eq!(txid1, txid1_again);
        assert_eq!(state.stats().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_add_transaction_capacity_limit() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(2, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Add two transactions (at capacity) - use sequential seq_no
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        state.add_transaction(&ctx, tx1).await.unwrap();
        state.add_transaction(&ctx, tx2).await.unwrap();

        // Third transaction should fail
        let tx3 = create_test_snark_tx_with_seq_no(3, 0);
        let result = state.add_transaction(&ctx, tx3).await;
        assert!(matches!(result, Err(OLMempoolError::MempoolFull { .. })));
    }

    #[tokio::test]
    async fn test_get_transactions_fifo_order() {
        use crate::test_utils::{
            create_test_account_id_with, create_test_attachment_with_slots,
            create_test_snark_tx_with_seq_no, setup_test_state_for_tip,
        };

        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Test 1: GenericAccountMessage transactions ordered by first_seen_slot
        // Use known account IDs that exist in test state (0-255)
        let account1 = create_test_account_id_with(200);
        let gam1 = OLMempoolTransaction::new_generic_account_message(
            account1,
            vec![1, 2, 3],
            create_test_attachment_with_slots(None, None),
        )
        .unwrap();
        let gam1_target = gam1.target();
        state.add_transaction(&ctx, gam1).await.unwrap();

        let tip101 = create_test_block_commitment(101);
        setup_test_state_for_tip(&ctx.storage, tip101).await;
        state.set_current_tip(tip101);
        let account2 = create_test_account_id_with(201);
        let gam2 = OLMempoolTransaction::new_generic_account_message(
            account2,
            vec![4, 5, 6],
            create_test_attachment_with_slots(None, None),
        )
        .unwrap();
        let gam2_target = gam2.target();
        state.add_transaction(&ctx, gam2).await.unwrap();

        let tip102 = create_test_block_commitment(102);
        setup_test_state_for_tip(&ctx.storage, tip102).await;
        state.set_current_tip(tip102);
        let account3 = create_test_account_id_with(202);
        let gam3 = OLMempoolTransaction::new_generic_account_message(
            account3,
            vec![7, 8, 9],
            create_test_attachment_with_slots(None, None),
        )
        .unwrap();
        let gam3_target = gam3.target();
        state.add_transaction(&ctx, gam3).await.unwrap();

        // GAM transactions should be ordered by slot (100 < 101 < 102)
        let txs = state.get_transactions(3).unwrap();
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].1.target(), gam1_target);
        assert_eq!(txs[1].1.target(), gam2_target);
        assert_eq!(txs[2].1.target(), gam3_target);

        // Clear mempool
        let all_txids: Vec<_> = txs.iter().map(|(txid, _)| *txid).collect();
        state.remove_transactions(&ctx, &all_txids).unwrap();

        // Test 2: SnarkAccountUpdate transactions ordered by seq_no
        // Use same account with sequential seq_no but different slots to verify seq_no ordering
        let account_id = 50; // Use account 50
        let snark1 = create_test_snark_tx_with_seq_no(account_id, 0);
        let snark2 = create_test_snark_tx_with_seq_no(account_id, 1);
        let snark3 = create_test_snark_tx_with_seq_no(account_id, 2);

        // Add in order at different slots: seq_no 0, 1, 2 (at slots 100, 101, 102)
        state.set_current_tip(create_test_block_commitment(100));
        state.add_transaction(&ctx, snark1).await.unwrap();

        state.set_current_tip(create_test_block_commitment(101));
        state.add_transaction(&ctx, snark2).await.unwrap();

        state.set_current_tip(create_test_block_commitment(102));
        state.add_transaction(&ctx, snark3).await.unwrap();

        // SnarkAccountUpdate transactions should be ordered by seq_no (0 < 1 < 2)
        let txs = state.get_transactions(3).unwrap();
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
    async fn test_gam_priority_same_slot() {
        use crate::test_utils::{create_test_account_id_with, create_test_attachment_with_slots};

        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Add three GAM transactions at the SAME slot (100)
        // They should get different priorities due to insertion_id
        let account1 = create_test_account_id_with(200);
        let gam1 = OLMempoolTransaction::new_generic_account_message(
            account1,
            vec![1, 2, 3],
            create_test_attachment_with_slots(None, None),
        )
        .unwrap();
        let gam1_target = gam1.target();
        state.add_transaction(&ctx, gam1).await.unwrap();

        let account2 = create_test_account_id_with(201);
        let gam2 = OLMempoolTransaction::new_generic_account_message(
            account2,
            vec![4, 5, 6],
            create_test_attachment_with_slots(None, None),
        )
        .unwrap();
        let gam2_target = gam2.target();
        state.add_transaction(&ctx, gam2).await.unwrap();

        let account3 = create_test_account_id_with(202);
        let gam3 = OLMempoolTransaction::new_generic_account_message(
            account3,
            vec![7, 8, 9],
            create_test_attachment_with_slots(None, None),
        )
        .unwrap();
        let gam3_target = gam3.target();
        state.add_transaction(&ctx, gam3).await.unwrap();

        // All three GAM transactions at same slot (100)
        // Should be ordered by insertion_id (FIFO order)
        let txs = state.get_transactions(3).unwrap();
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].1.target(), gam1_target); // First inserted
        assert_eq!(txs[1].1.target(), gam2_target); // Second inserted
        assert_eq!(txs[2].1.target(), gam3_target); // Third inserted
    }

    #[tokio::test]
    async fn test_snark_priority_different_accounts_same_seq_no() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Add three SnarkAccountUpdate transactions from DIFFERENT accounts
        // All with seq_no=0 (valid for each account)
        // They should get different priorities due to insertion_id
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1_target = tx1.target();
        state.add_transaction(&ctx, tx1).await.unwrap();

        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        let tx2_target = tx2.target();
        state.add_transaction(&ctx, tx2).await.unwrap();

        let tx3 = create_test_snark_tx_with_seq_no(3, 0);
        let tx3_target = tx3.target();
        state.add_transaction(&ctx, tx3).await.unwrap();

        // All three transactions have seq_no=0 but different accounts
        // Should be ordered by insertion_id (FIFO order)
        let txs = state.get_transactions(3).unwrap();
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].1.target(), tx1_target); // First inserted
        assert_eq!(txs[1].1.target(), tx2_target); // Second inserted
        assert_eq!(txs[2].1.target(), tx3_target); // Third inserted
    }

    #[tokio::test]
    async fn test_snark_priority_same_account_gap_rejected() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Add first transaction with seq_no=0 for account 1
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        state.add_transaction(&ctx, tx1).await.unwrap();

        // Try to add another transaction with seq_no=0 for same account
        // This should be REJECTED due to gap checking (expected seq_no=1, got 0)
        let tx2 = create_test_snark_tx_with_seq_no(1, 0);
        let result = state.add_transaction(&ctx, tx2).await;
        assert!(matches!(
            result,
            Err(OLMempoolError::SequenceNumberGap {
                expected: 1,
                actual: 0
            })
        ));

        // Only one transaction should be in mempool
        assert_eq!(state.stats().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_gap_rejection() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Add transaction with seq_no=0 for account 1
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        state.add_transaction(&ctx, tx1).await.unwrap();

        // Try to add transaction with seq_no=2 (gap - missing seq_no=1)
        // Should be REJECTED
        let tx3 = create_test_snark_tx_with_seq_no(1, 2);
        let result = state.add_transaction(&ctx, tx3).await;
        assert!(matches!(
            result,
            Err(OLMempoolError::SequenceNumberGap {
                expected: 1,
                actual: 2
            })
        ));

        // Now add seq_no=1 (correct sequential order)
        let tx2 = create_test_snark_tx_with_seq_no(1, 1);
        state.add_transaction(&ctx, tx2).await.unwrap();

        // Now we can add seq_no=2
        let tx3_retry = create_test_snark_tx_with_seq_no(1, 2);
        state.add_transaction(&ctx, tx3_retry).await.unwrap();

        // Should have 3 transactions now (0, 1, 2)
        assert_eq!(state.stats().mempool_size(), 3);
    }

    #[tokio::test]
    async fn test_get_transactions_limit() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Add 5 transactions - each to different account with seq_no 0
        for i in 1..=5 {
            let tx = create_test_snark_tx_with_seq_no(i, 0);
            state.add_transaction(&ctx, tx).await.unwrap();
        }

        // Request only 3
        let txs = state.get_transactions(3).unwrap();
        assert_eq!(txs.len(), 3);
    }

    #[tokio::test]
    async fn test_remove_transactions() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Add transactions - each to different account with seq_no 0
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        let txid1 = state.add_transaction(&ctx, tx1.clone()).await.unwrap();
        let txid2 = state.add_transaction(&ctx, tx2.clone()).await.unwrap();

        assert_eq!(state.stats().mempool_size(), 2);

        // Remove one transaction
        let removed = state.remove_transactions(&ctx, &[txid1]).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], txid1);

        // Should be gone
        assert!(!state.contains(&txid1));
        assert!(state.contains(&txid2));
        assert_eq!(state.stats().mempool_size(), 1);
    }

    #[test]
    fn test_remove_nonexistent_transaction() {
        let ctx = create_test_context(10, 1_000_000);
        let mut state = MempoolState::new(create_test_block_commitment(100));

        // Remove transaction that doesn't exist - should succeed with empty result
        let fake_txid = OLTxId::from(Buf32::from([0u8; 32]));
        let removed = state.remove_transactions(&ctx, &[fake_txid]).unwrap();
        assert_eq!(removed.len(), 0);
    }

    #[tokio::test]
    async fn test_load_from_db() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Add transactions - each to different account with seq_no 0
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        state.add_transaction(&ctx, tx1).await.unwrap();
        state.add_transaction(&ctx, tx2).await.unwrap();

        // Create new state and load from DB
        let mut state2 = MempoolState::new(tip);
        state2.load_from_db(&ctx).unwrap();

        // Should have same transactions
        assert_eq!(state2.stats().mempool_size(), 2);
    }

    #[tokio::test]
    async fn test_stats_updates() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        let initial_stats = state.stats();
        assert_eq!(initial_stats.mempool_size(), 0);
        assert_eq!(initial_stats.total_bytes(), 0);
        assert_eq!(initial_stats.enqueues_accepted(), 0);

        // Add first transaction - account 1 with seq_no 0
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1_size = ssz::Encode::as_ssz_bytes(&tx1).len();
        state.add_transaction(&ctx, tx1.clone()).await.unwrap();

        let stats_after_first = state.stats();
        assert_eq!(stats_after_first.mempool_size(), 1);
        assert_eq!(stats_after_first.total_bytes(), tx1_size);
        assert_eq!(stats_after_first.enqueues_accepted(), 1);

        // Add second transaction - account 2 with seq_no 0
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        let tx2_size = ssz::Encode::as_ssz_bytes(&tx2).len();
        state.add_transaction(&ctx, tx2).await.unwrap();

        let stats_after_second = state.stats();
        assert_eq!(stats_after_second.mempool_size(), 2);
        assert_eq!(stats_after_second.total_bytes(), tx1_size + tx2_size);
        assert_eq!(stats_after_second.enqueues_accepted(), 2);

        // Idempotent add (should not increment enqueues_accepted again)
        state.add_transaction(&ctx, tx1).await.unwrap();

        let stats_after_idempotent = state.stats();
        assert_eq!(stats_after_idempotent.mempool_size(), 2);
        assert_eq!(stats_after_idempotent.enqueues_accepted(), 2);
    }

    #[tokio::test]
    async fn test_stats_rejections() {
        let tip = create_test_block_commitment(100);
        let ctx = create_test_context_with_state(2, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

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
        state.add_transaction(&ctx, tx1).await.unwrap();
        state.add_transaction(&ctx, tx2).await.unwrap();

        // Try to add when full
        let tx3 = create_test_snark_tx_with_seq_no(3, 0);
        let result = state.add_transaction(&ctx, tx3).await;
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
        let ctx_tiny = create_test_context_with_state(10, 50, tip2).await;
        let mut state2 = MempoolState::new(tip2);

        let large_tx = create_test_tx_with_id(99);
        let result = state2.add_transaction(&ctx_tiny, large_tx).await;
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
        let ctx = create_test_context_with_state(10, 1_000_000, tip).await;
        let mut state = MempoolState::new(tip);

        // Add transactions for account 1: seq_no 0, 1, 2
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);

        let txid0 = state.add_transaction(&ctx, tx0).await.unwrap();
        let txid1 = state.add_transaction(&ctx, tx1).await.unwrap();
        let txid2 = state.add_transaction(&ctx, tx2).await.unwrap();

        assert_eq!(state.stats().mempool_size(), 3);

        // Remove middle transaction (seq_no 1) - creates gap!
        let removed = state.remove_transactions(&ctx, &[txid1]).unwrap();

        // Should remove tx1 AND tx2 (cascade due to gap)
        assert_eq!(removed.len(), 2); // Both tx1 and tx2 removed
        assert!(removed.contains(&txid1));
        assert!(removed.contains(&txid2));

        // Only tx0 should remain
        assert_eq!(state.stats().mempool_size(), 1);
        assert!(state.contains(&txid0));
        assert!(!state.contains(&txid1));
        assert!(!state.contains(&txid2));

        // Verify pending_seq_no tracking is correct by adding next transaction
        // If pending_seq_no is correct (should be 1), this should succeed
        let tx_next = create_test_snark_tx_with_seq_no(1, 1);
        let result = state.add_transaction(&ctx, tx_next).await;
        assert!(result.is_ok(), "Should accept seq_no 1 after gap removal");
    }
}
