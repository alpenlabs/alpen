//! Mempool service state management.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::mem;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use metrics::{counter, gauge};
use ssz::{Decode, Encode};
use strata_acct_types::AccountId;
use strata_bridge_params::BridgeParams;
use strata_db_types::mempool::{MempoolTxData, MempoolWriteBatch};
use strata_identifiers::{L1BlockCommitment, OLBlockCommitment, OLTxId};
use strata_ol_chain_types_v1::OLBlockV1;
use strata_ol_log_budget::check_tx_log_budget;
use strata_ol_spec_selection::SelectedSpec;
use strata_ol_state_provider::{OLStateManagerProviderImpl, StateProvider};
use strata_ol_state_types::OLSpecId;
use strata_ol_stf::{UpgradeRequired, spec_limits};
use strata_ol_tx_types_v1::{OLTransactionV1, TransactionPayloadV1};
use strata_service::ServiceState;
use strata_storage::{MempoolDbManager, NodeStorage};
use tracing::{debug, info, instrument, warn};

use crate::tip::{MempoolTip, select_tip_spec};
use crate::types::{MempoolEntry, OLMempoolConfig, OLMempoolRejectReason, OLMempoolStats};
use crate::validation::validate_transaction;
use crate::{MempoolCandidates, MempoolTxInvalidReason, OLMempoolError, OLMempoolResult};

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

    /// Network parameters shared with the STF's bridge-message classification.
    pub(crate) bridge_params: BridgeParams,

    /// Storage backend for database operations (transactions, blocks).
    pub(crate) storage: Arc<NodeStorage>,

    /// Mempool transaction database, the one in [`Self::storage`] outside
    /// tests, which replace it to inject storage failures.
    pub(crate) mempool_db: Arc<MempoolDbManager>,

    /// State provider for fetching OL state at different chain tips.
    pub(crate) provider: Arc<P>,

    /// L1 block OL genesis anchors to, where the genesis epoch's L1 range
    /// starts for spec selection.
    pub(crate) genesis_l1_block: L1BlockCommitment,
}

impl<P: StateProvider> MempoolContext<P> {
    /// Create new mempool context with explicit provider.
    pub(crate) fn new(
        config: OLMempoolConfig,
        bridge_params: BridgeParams,
        storage: Arc<NodeStorage>,
        provider: Arc<P>,
        genesis_l1_block: L1BlockCommitment,
    ) -> MempoolContext<P> {
        MempoolContext {
            config,
            bridge_params,
            mempool_db: storage.mempool().clone(),
            storage,
            provider,
            genesis_l1_block,
        }
    }
}

impl MempoolContext<OLStateManagerProviderImpl> {
    /// Create new mempool context.
    ///
    /// Extracts the state provider from storage.
    pub(crate) fn new_from_nodestorage(
        config: OLMempoolConfig,
        bridge_params: BridgeParams,
        storage: Arc<NodeStorage>,
        genesis_l1_block: L1BlockCommitment,
    ) -> Self {
        let provider = Arc::new(OLStateManagerProviderImpl::new(storage.ol_state().clone()));
        Self {
            config,
            bridge_params,
            mempool_db: storage.mempool().clone(),
            storage,
            provider,
            genesis_l1_block,
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

    /// Per-account mempool state.
    /// Tracks all txids and sequence numbers for each account.
    account_state: HashMap<AccountId, AccountMempoolState>,

    /// Chain tip the mempool admits transactions for. Chain updates find the
    /// blocks they revert and adopt from it.
    tip: MempoolTip,

    /// State accessor for validation. Updated when chain tip changes.
    state_accessor: P::State,

    /// Spec of the block after the tip, selected with the tip state.
    tip_spec: SelectedSpec,

    /// Whether loading the stored transactions waits for a tip whose next
    /// block this binary can run.
    deferred_load: bool,

    /// Whether memory holds stored transactions nothing has validated yet,
    /// which the next chain update readmits even if the tip did not move.
    needs_readmit: bool,

    /// Whether the last chain update failed. The mempool then cannot tell
    /// which held transactions the chain includes, so it serves no candidates
    /// and admits nothing until an update succeeds.
    out_of_sync: bool,

    /// Mempool statistics.
    stats: OLMempoolStats,
}

impl<P: StateProvider> MempoolServiceState<P> {
    /// Create new mempool service state.
    #[expect(dead_code, reason = "another constructor is used")]
    pub(crate) async fn new(
        config: OLMempoolConfig,
        bridge_params: BridgeParams,
        storage: Arc<NodeStorage>,
        provider: Arc<P>,
        genesis_l1_block: L1BlockCommitment,
        tip: MempoolTip,
    ) -> OLMempoolResult<Self> {
        let ctx = Arc::new(MempoolContext::new(
            config,
            bridge_params,
            storage,
            provider,
            genesis_l1_block,
        ));
        Self::new_with_context(ctx, tip).await
    }

    /// Create new mempool service state with an existing context.
    /// Used for testing.
    ///
    /// Fetches the state for the given tip from the provider and selects the
    /// spec of the block after it.
    pub(crate) async fn new_with_context(
        ctx: Arc<MempoolContext<P>>,
        tip: MempoolTip,
    ) -> OLMempoolResult<Self> {
        let (state_accessor, tip_spec) = load_tip(&ctx, tip).await?;
        log_upgrade_required(tip.block(), None, &tip_spec);

        let state = Self {
            ctx,
            entries: HashMap::new(),
            account_state: HashMap::new(),
            tip,
            state_accessor,
            tip_spec,
            deferred_load: false,
            needs_readmit: false,
            out_of_sync: false,
            stats: OLMempoolStats::default(),
        };
        state.record_mempool_gauges();

        Ok(state)
    }

    /// Loads the stored transactions and catches the mempool up from the tip
    /// its database last recorded to the current tip, then readmits
    /// everything against the current tip.
    ///
    /// The node can stop after a block lands and before the mempool commits
    /// the chain update for it, so the database can still hold transactions
    /// that block includes, or lack ones a reorg since then reverted. A
    /// generic account message carries no sequence number, so readmitting an
    /// included one would let block assembly apply it twice. Each chain
    /// update records its tip in the same batch as its writes, so the
    /// recorded tip is the last tip the stored transactions reflect.
    ///
    /// Loading does not validate, because the stored transactions reflect the
    /// recorded tip, not the current one: an update whose predecessor a reorg
    /// reverted would look like a gap until the catch-up restores the
    /// predecessor. The catch-up is the chain update from the recorded tip to
    /// the current tip, which removes what the blocks in between include,
    /// restores what a reorg reverted, or evicts everything if the two tips
    /// share no ancestor within the maximum reorg depth. It then readmits the
    /// loaded and restored transactions once, in account order, and writes the
    /// result and the current tip in one batch. Without a recorded tip, as on
    /// the first launch after an upgrade, it readmits against the current tip
    /// and records it.
    ///
    /// When the next block needs an upgrade, nothing is loaded until a tip's
    /// next block runs a spec this binary implements, as
    /// [`Self::load_stored`] describes.
    ///
    /// A failed catch-up leaves the mempool out of sync at the recorded tip,
    /// and the next chain update retries it.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the stored transactions or the recorded tip
    /// fails.
    pub(crate) async fn load_from_db(&mut self) -> OLMempoolResult<()> {
        self.load_stored()?;
        let current_tip = self.tip;
        if let Some(recorded) = self.ctx.mempool_db.get_tip()?
            && recorded != current_tip.block()
        {
            info!(
                %recorded,
                current_tip = %current_tip.block(),
                "catching the mempool up from its recorded tip"
            );
            self.tip = MempoolTip::new(recorded, None);
        }
        // A failure is logged and leaves the mempool out of sync.
        let _ = self.handle_chain_update(current_tip).await;
        Ok(())
    }

    /// Loads the stored transactions into memory without validating them, and
    /// marks them for the readmission the next chain update runs.
    ///
    /// Malformed rows are skipped and deleted. When the next block needs an
    /// upgrade, admission cannot run, so nothing is loaded: the stored
    /// transactions stay in the database until a chain update reaches a tip
    /// whose next block runs a spec this binary implements, which loads and
    /// readmits them.
    ///
    /// # Errors
    ///
    /// Returns an error if listing the stored transactions fails.
    #[instrument(skip(self), fields(component = "ol_mempool"))]
    fn load_stored(&mut self) -> OLMempoolResult<()> {
        let all_txs = self.ctx.mempool_db.get_all_txs()?;
        let total_in_db = all_txs.len();
        if let SelectedSpec::Unsupported(upgrade) = &self.tip_spec {
            debug!(
                %total_in_db,
                %upgrade,
                "not loading mempool transactions until this node is upgraded"
            );
            self.deferred_load = true;
            return Ok(());
        }
        self.deferred_load = false;
        info!(%total_in_db, "loading mempool transactions from database");

        let mut loaded_count = 0usize;
        for tx_data in all_txs {
            if self.contains(&tx_data.txid()) {
                continue;
            }
            let tx: OLTransactionV1 = match Decode::from_ssz_bytes(tx_data.tx_bytes()) {
                Ok(tx) => tx,
                Err(e) => {
                    // Skip malformed transaction and remove from DB
                    warn!(
                        txid = ?tx_data.txid(),
                        ?e,
                        "Skipping malformed transaction from database"
                    );
                    if let Err(del_err) = self.ctx.mempool_db.del_tx(tx_data.txid()) {
                        warn!(
                            txid = ?tx_data.txid(),
                            error = %del_err,
                            "failed to delete malformed tx from mempool db; will retry on next reload"
                        );
                    }
                    continue;
                }
            };
            let entry = MempoolEntry::new(tx, tx_data.timestamp_micros(), tx_data.tx_bytes().len());
            self.add_tx_to_in_memory_state(tx_data.txid(), entry);
            loaded_count += 1;
        }
        self.stats.enqueues_accepted += loaded_count as u64;
        self.needs_readmit = true;
        debug!(%loaded_count, "loaded mempool transactions for readmission");
        Ok(())
    }

    /// Handle submit transaction command.
    #[instrument(skip(self, tx), fields(component = "ol_mempool"))]
    pub(crate) async fn handle_submit_transaction(
        &mut self,
        tx: Box<OLTransactionV1>,
    ) -> OLMempoolResult<OLTxId> {
        // Add to mempool
        let txid = self.add_transaction(*tx).await?;
        Ok(txid)
    }

    /// Shares the current transaction bodies with one candidate snapshot.
    ///
    /// The snapshot is empty while the mempool is out of sync, since a held
    /// transaction may be one the chain already includes.
    pub(crate) fn handle_get_candidates(&self) -> MempoolCandidates {
        if self.out_of_sync {
            debug!("serving no candidates while out of sync with the chain");
            return MempoolCandidates::default();
        }
        MempoolCandidates::build_snapshot(
            self.entries
                .iter()
                .map(|(&txid, entry)| (txid, &entry.tx, entry.timestamp_micros)),
        )
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
    fn find_account_tx_with_same_seqno(&self, tx: &OLTransactionV1) -> Option<OLTxId> {
        let target_account = tx.target()?;
        let tx_seq_no = match tx.payload() {
            TransactionPayloadV1::SnarkAccountUpdate(payload) => {
                payload.operation().update().seq_no()
            }
            TransactionPayloadV1::GenericAccountMessage(_) => return None,
        };

        // Get account state and check if seq_no exists
        let acct_state = self.account_state.get(&target_account)?;
        let (min_seq_no, max_seq_no) = acct_state.seq_no_range()?;

        if tx_seq_no < min_seq_no || tx_seq_no > max_seq_no {
            return None;
        }

        // Find txid with this seq_no in the account's transactions
        for txid in &acct_state.txids {
            if let Some(entry) = self.entries.get(txid) {
                if let TransactionPayloadV1::SnarkAccountUpdate(payload) = entry.tx.payload()
                    && payload.operation().update().seq_no() == tx_seq_no
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

    /// Returns the spec of the block after the tip, the block admitted
    /// transactions are built into.
    ///
    /// # Errors
    ///
    /// Returns [`OLMempoolError::UpgradeRequired`] if that block runs a spec
    /// this binary does not implement.
    fn select_spec(&self) -> OLMempoolResult<OLSpecId> {
        Ok(self.tip_spec.require_supported()?)
    }

    /// Checks whether a submitted or restored transaction can enter the mempool.
    fn validate_admission(&self, txid: OLTxId, tx: &OLTransactionV1) -> OLMempoolResult<()> {
        let spec = self.select_spec()?;
        spec_limits(spec).check_tx_effects(tx.data().effects())?;
        check_tx_log_budget(spec, tx, &self.ctx.bridge_params)?;
        validate_transaction(spec, txid, tx, &self.state_accessor, &self.account_state)
    }

    /// Add a transaction to the mempool.
    ///
    /// Returns the transaction ID. Idempotent - returns existing txid if duplicate.
    #[instrument(skip(self, tx), fields(component = "ol_mempool"))]
    pub(crate) async fn add_transaction(&mut self, tx: OLTransactionV1) -> OLMempoolResult<OLTxId> {
        let txid = tx.compute_txid();

        if self.out_of_sync {
            debug!(
                ?txid,
                "rejecting transaction while out of sync with the chain"
            );
            return Err(OLMempoolError::OutOfSync);
        }

        // Nothing is admitted while the next block needs an upgrade, not even
        // a transaction the mempool already holds.
        self.select_spec()?;

        // Idempotent check - if already present, return success
        if self.contains(&txid) {
            self.update_stats_on_reject(OLMempoolRejectReason::Duplicate);
            debug!(?txid, "rejecting duplicate transaction");
            return Ok(txid);
        }

        // Encode transaction once for both size validation and database persistence
        let tx_bytes = Encode::as_ssz_bytes(&tx);
        let tx_size = tx_bytes.len();

        // Check individual transaction size limit
        if tx_size > self.ctx.config.max_tx_size {
            self.update_stats_on_reject(OLMempoolRejectReason::TransactionTooLarge);
            debug!(
                ?txid,
                %tx_size,
                limit = %self.ctx.config.max_tx_size,
                "rejecting transaction: too large"
            );
            return Err(OLMempoolError::TransactionTooLarge {
                size: tx_size,
                limit: self.ctx.config.max_tx_size,
            });
        }

        if let Err(e) = self.validate_admission(txid, &tx) {
            if let Some(reason) = OLMempoolRejectReason::from_error(&e) {
                self.update_stats_on_reject(reason);
            }
            debug!(?txid, error = ?e, "rejecting transaction: validation failed");
            return Err(e);
        }

        // Check mempool capacity limits
        if let Err(e) = self.check_capacity_limits(tx_size) {
            self.update_stats_on_reject(OLMempoolRejectReason::MempoolFull);
            debug!(?txid, error = ?e, "rejecting transaction: capacity limit");
            return Err(e);
        }

        // Check if this is a replacement transaction and remove old one if needed
        if let Some(old_txid) = self.find_account_tx_with_same_seqno(&tx) {
            debug!(
                ?txid,
                replaced = ?old_txid,
                "replacing transaction with same seq_no (last-write-wins)"
            );
            // Remove old tx with same sequence number (last-write-wins), no cascade
            self.remove_transactions(&[old_txid], false);
        }

        let timestamp_micros = now_micros();
        let entry = MempoolEntry::new(tx, timestamp_micros, tx_size);

        // Persist to database first
        let tx_data = MempoolTxData::new(txid, tx_bytes, timestamp_micros);
        self.ctx.mempool_db.put_tx(tx_data)?;

        // Add to in-memory state
        self.add_tx_to_in_memory_state(txid, entry);
        self.stats.enqueues_accepted += 1;
        counter!("strata_mempool_enqueued_total").increment(1);

        debug!(
            ?txid,
            tx_size,
            mempool_size = %self.stats.mempool_size(),
            "accepted transaction into mempool"
        );

        Ok(txid)
    }

    /// Add a transaction to in-memory state.
    ///
    /// Updates all in-memory data structures:
    /// - entries: Main transaction storage
    /// - account_state: Per-account tracking for validation
    ///
    /// Also updates the size statistics, but not the accepted count, which
    /// depends on whether the caller admits a new transaction or re-admits a
    /// held one. Does NOT write to database or perform validation.
    fn add_tx_to_in_memory_state(&mut self, txid: OLTxId, entry: MempoolEntry) {
        let target_account = entry
            .tx
            .target()
            .expect("all OL payload variants must have a target");
        let tx_size = entry.size_bytes;

        // Add to entries
        self.entries.insert(txid, entry.clone());

        // Add to account_state index
        let acct_state = self.account_state.entry(target_account).or_default();
        acct_state.txids.insert(txid);

        // Add seq_no if this is a SnarkAccountUpdate
        if let TransactionPayloadV1::SnarkAccountUpdate(payload) = entry.tx.payload() {
            acct_state
                .seq_nos
                .insert(payload.operation().update().seq_no());
        }

        // Update stats
        self.update_stats_on_add(tx_size);
    }

    /// Helper to remove a single transaction from all internal data structures.
    fn remove_single_tx(&mut self, txid: OLTxId, entry: &MempoolEntry) -> OLMempoolResult<()> {
        debug!(
            ?txid,
            size = %entry.size_bytes,
            "removing transaction from mempool"
        );

        // Remove from database first
        self.ctx.mempool_db.del_tx(txid)?;
        self.remove_from_memory(txid, entry);
        Ok(())
    }

    /// Removes a transaction from the in-memory state only.
    fn remove_from_memory(&mut self, txid: OLTxId, entry: &MempoolEntry) {
        let size_bytes = entry.size_bytes;
        let account_id = entry
            .tx
            .target()
            .expect("all OL payload variants must have a target");

        // Remove from memory
        self.entries.remove(&txid);

        // Remove from account_state index
        if let Some(acct_state) = self.account_state.get_mut(&account_id) {
            acct_state.txids.remove(&txid);

            // Remove seq_no if this is a SnarkAccountUpdate
            if let TransactionPayloadV1::SnarkAccountUpdate(payload) = entry.tx.payload() {
                acct_state
                    .seq_nos
                    .remove(&payload.operation().update().seq_no());
            }

            // Remove account state entirely if no more transactions
            if acct_state.txids.is_empty() {
                self.account_state.remove(&account_id);
            }
        }

        // Update stats
        self.update_stats_on_remove(size_bytes);
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
                let account = entry
                    .tx
                    .target()
                    .expect("all OL payload variants must have a target");

                // Remove using helper
                self.remove_single_tx(*txid, &entry)?;

                // Track minimum seq_no for account (for cascade removal)
                if let TransactionPayloadV1::SnarkAccountUpdate(payload) = entry.tx.payload() {
                    let seq_no = payload.operation().update().seq_no();
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
                match entry.tx.payload() {
                    TransactionPayloadV1::SnarkAccountUpdate(payload) => {
                        let seq_no = payload.operation().update().seq_no();
                        (seq_no >= min_failed_seq).then_some(txid)
                    }
                    TransactionPayloadV1::GenericAccountMessage(_) => None,
                }
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

    /// Update stats when a transaction enters the in-memory state.
    fn update_stats_on_add(&mut self, tx_size: usize) {
        self.stats.mempool_size += 1;
        self.stats.total_bytes += tx_size;
        self.record_mempool_gauges();
    }

    /// Update stats when a transaction is removed.
    fn update_stats_on_remove(&mut self, tx_size: usize) {
        self.stats.mempool_size -= 1;
        self.stats.total_bytes -= tx_size;
        self.record_mempool_gauges();
    }

    /// Update stats when a transaction is rejected.
    fn update_stats_on_reject(&mut self, reason: OLMempoolRejectReason) {
        self.stats.enqueues_rejected += 1;
        self.stats.rejects_by_reason.increment(reason);
        counter!("strata_mempool_rejected_total", "reason" => reason.as_str()).increment(1);
    }

    fn record_mempool_gauges(&self) {
        gauge!("strata_mempool_tx_count").set(self.stats.mempool_size() as f64);
        gauge!("strata_mempool_size_bytes").set(self.stats.total_bytes() as f64);
    }

    /// Admits `pending` in account sequence order against the tip state and
    /// returns the transactions that fail admission.
    ///
    /// The mempool checks an update against the account's pending range when
    /// it holds updates for the account, and against the chain state only
    /// otherwise. Admitting from an empty account index in sequence order
    /// therefore checks each account's first update against the chain and the
    /// rest for contiguity.
    ///
    /// Each transaction must also fit the count and byte limits, as a
    /// submission must. Admission follows arrival order, with each account's
    /// updates in sequence order, so once the mempool is full every later
    /// transaction that does not fit is rejected, and a rejected update's
    /// successors fail for the gap it leaves. A later, smaller transaction may
    /// still fit the byte limit.
    ///
    /// Admitted transactions only enter the in-memory state; the caller owns
    /// persistence and the accepted count.
    fn admit_in_account_order(
        &mut self,
        mut pending: HashMap<OLTxId, MempoolEntry>,
    ) -> Vec<(OLTxId, OLMempoolError)> {
        let candidates = MempoolCandidates::build_snapshot(
            pending
                .iter()
                .map(|(&txid, entry)| (txid, &entry.tx, entry.timestamp_micros)),
        );
        let mut rejected = Vec::new();
        for candidate in candidates {
            let txid = candidate.txid();
            let entry = pending
                .remove(&txid)
                .expect("candidate was taken from the pending set");
            let admission = self
                .validate_admission(txid, &entry.tx)
                .and_then(|()| self.check_capacity_limits(entry.size_bytes));
            match admission {
                Ok(()) => self.add_tx_to_in_memory_state(txid, entry),
                Err(e) => rejected.push((txid, e)),
            }
        }
        rejected
    }

    /// Re-admits every held transaction together with `restored`, the
    /// transactions of reverted blocks, against the tip state, and returns
    /// how many restored transactions it admitted.
    ///
    /// The chain state can move an account's sequence number away from the
    /// updates the mempool holds for it: an extension may include a
    /// conflicting update, and a reorg can lower it below the held ones,
    /// which then only stay valid behind the restored updates. Readmitting
    /// everything in account order with [`Self::admit_in_account_order`]
    /// checks each account's first update against the chain, restores the
    /// dependency order, and evicts whatever the new state invalidates, along
    /// with its successors.
    ///
    /// Restored transactions count against the capacity limits like any
    /// admission. They arrived before everything the mempool holds, so a
    /// mempool that cannot fit both keeps the restored transactions and evicts
    /// its most recently arrived held ones.
    ///
    /// When the next block needs an upgrade, admission cannot run. The
    /// restored transactions are then held without validation, like every
    /// other held transaction, for a binary that implements the next spec, and
    /// the reorg or restart that resumes admission revalidates them.
    ///
    /// This only changes memory. It records the rows to delete for evicted
    /// transactions and the rows to store for admitted restored ones in
    /// `changes`, which the chain update writes before it commits.
    fn readmit_against_tip(
        &mut self,
        restored: Vec<(OLTxId, MempoolEntry)>,
        changes: &mut StorageChanges,
    ) -> usize {
        if !matches!(self.tip_spec, SelectedSpec::Supported(_)) {
            let restored_count = restored.len();
            for (txid, entry) in restored {
                changes.puts.push(txid);
                self.add_tx_to_in_memory_state(txid, entry);
            }
            return restored_count;
        }

        self.needs_readmit = false;
        let restored_txids: HashSet<OLTxId> = restored.iter().map(|(txid, _)| *txid).collect();
        let mut pending = mem::take(&mut self.entries);
        self.account_state.clear();
        self.stats.mempool_size = 0;
        self.stats.total_bytes = 0;
        pending.extend(restored);

        let rejected = self.admit_in_account_order(pending);
        let mut dropped_restored = 0usize;
        for (txid, e) in &rejected {
            if matches!(
                e,
                OLMempoolError::MempoolFull { .. }
                    | OLMempoolError::MempoolByteLimitExceeded { .. }
            ) {
                self.stats.evictions += 1;
            }
            if restored_txids.contains(txid) {
                dropped_restored += 1;
                debug!(
                    ?txid,
                    ?e,
                    "dropping reverted transaction invalid on the new tip"
                );
            } else {
                debug!(?txid, ?e, "evicting transaction the new tip invalidates");
                changes.deletes.insert(*txid);
            }
        }

        let restored_count = restored_txids.len() - dropped_restored;
        changes.puts.extend(
            restored_txids
                .into_iter()
                .filter(|txid| self.entries.contains_key(txid)),
        );
        self.stats.enqueues_accepted += restored_count as u64;
        self.record_mempool_gauges();

        debug!(
            %restored_count,
            %dropped_restored,
            evicted = rejected.len() - dropped_restored,
            "readmitted mempool transactions against the new tip"
        );
        restored_count
    }

    /// Writes `changes` and `tip` to the mempool database in one atomic
    /// batch: the deletes, the restored transactions memory now holds, and
    /// the tip they reflect, which a restart catches up from.
    ///
    /// # Errors
    ///
    /// Returns the storage error if the batch fails, in which case it wrote
    /// nothing.
    fn apply_storage_changes(
        &self,
        changes: &StorageChanges,
        tip: OLBlockCommitment,
    ) -> OLMempoolResult<()> {
        let mut batch = MempoolWriteBatch::default();
        for txid in &changes.deletes {
            batch.delete(*txid);
        }
        for txid in &changes.puts {
            let entry = self
                .entries
                .get(txid)
                .expect("restored transactions to store are held");
            batch.put(MempoolTxData::new(
                *txid,
                Encode::as_ssz_bytes(entry.tx.as_ref()),
                entry.timestamp_micros,
            ));
        }
        batch.set_tip(tip);
        self.ctx.mempool_db.apply_batch(batch)?;
        Ok(())
    }

    /// Collects the transactions of `reverted`, oldest block first, that the
    /// mempool should restore.
    ///
    /// Skips transactions the adopted branch includes, ones the mempool already
    /// holds, updates whose account and sequence number a held or earlier
    /// restored update already takes, and ones over the size limit. Restored
    /// transactions arrived before everything the mempool holds, so they get
    /// timestamps just before the oldest held one, in chain order.
    fn collect_restored(
        &self,
        reverted: &[OLBlockV1],
        adopted_txids: &HashSet<OLTxId>,
    ) -> Vec<(OLTxId, MempoolEntry)> {
        let mut taken_seq_nos: HashSet<(AccountId, u64)> = self
            .entries
            .values()
            .filter_map(|entry| snark_update_key(&entry.tx))
            .collect();

        let mut restored = Vec::new();
        for block in reverted {
            let Some(tx_segment) = block.body().tx_segment() else {
                continue;
            };
            for tx in tx_segment.txs() {
                let tx = tx.clone().with_accumulator_proofs(None);
                let txid = tx.compute_txid();
                if adopted_txids.contains(&txid) || self.contains(&txid) {
                    continue;
                }
                if let Some(key) = snark_update_key(&tx)
                    && !taken_seq_nos.insert(key)
                {
                    debug!(?txid, "not restoring reverted update whose seq_no is taken");
                    continue;
                }
                let tx_size = tx.ssz_bytes_len();
                if tx_size > self.ctx.config.max_tx_size {
                    debug!(?txid, %tx_size, "not restoring oversized reverted transaction");
                    continue;
                }
                restored.push((txid, tx, tx_size));
            }
        }

        let oldest_held = self
            .entries
            .values()
            .map(|entry| entry.timestamp_micros)
            .min()
            .unwrap_or(u64::MAX);
        let base = oldest_held.min(now_micros());
        let count = restored.len() as u64;
        restored
            .into_iter()
            .zip(0..)
            .map(|((txid, tx, tx_size), index)| {
                let timestamp_micros = base.saturating_sub(count - index);
                (txid, MempoolEntry::new(tx, timestamp_micros, tx_size))
            })
            .collect()
    }

    /// Evicts every held transaction from memory, records every row of the
    /// mempool database for deletion in `changes`, and returns how many
    /// transactions it evicted.
    ///
    /// A chain update without a common ancestor leaves the mempool unable to
    /// tell which held transactions the new branch already includes. Generic
    /// account messages carry no sequence number, so revalidation cannot catch
    /// one the branch included, and keeping it could apply it twice. The
    /// database rows go too, including ones a deferred load has not read yet,
    /// so a reload cannot bring them back.
    ///
    /// # Errors
    ///
    /// Returns the storage error if listing the rows fails, before memory
    /// changes.
    fn evict_all(&mut self, changes: &mut StorageChanges) -> OLMempoolResult<usize> {
        let mut evicted: HashSet<OLTxId> = self.entries.keys().copied().collect();
        let stored = self.ctx.mempool_db.get_all_txs()?;
        evicted.extend(stored.iter().map(|tx_data| tx_data.txid()));

        self.entries.clear();
        self.account_state.clear();
        self.stats.mempool_size = 0;
        self.stats.total_bytes = 0;
        self.deferred_load = false;
        self.needs_readmit = false;

        let count = evicted.len();
        self.stats.reorg_evictions += count as u64;
        changes.deletes.extend(evicted);
        self.record_mempool_gauges();
        Ok(count)
    }

    /// Removes the transactions `blocks` include from memory, without
    /// cascade, records their rows for deletion in `changes`, and returns
    /// their IDs.
    ///
    /// The rows include ones memory does not hold, which a deferred load would
    /// otherwise read back. A generic account message carries no sequence
    /// number, so a row left behind could reload and apply it twice.
    ///
    /// # Errors
    ///
    /// Returns an error if a block cannot be read, before memory changes.
    async fn remove_included(
        &mut self,
        blocks: &[OLBlockCommitment],
        changes: &mut StorageChanges,
    ) -> OLMempoolResult<HashSet<OLTxId>> {
        let mut included = HashSet::new();
        for &commitment in blocks {
            let block = self.get_block(commitment).await?;
            included.extend(
                extract_account_txs_from_block(&block)
                    .into_values()
                    .flatten(),
            );
        }
        let held: Vec<(OLTxId, MempoolEntry)> = included
            .iter()
            .filter_map(|txid| Some((*txid, self.entries.get(txid)?.clone())))
            .collect();
        if !held.is_empty() {
            debug!(
                included_count = %held.len(),
                "removing block-included transactions from mempool"
            );
        }
        for (txid, entry) in &held {
            self.remove_from_memory(*txid, entry);
        }
        changes.deletes.extend(included.iter().copied());
        Ok(included)
    }

    /// Fetches the block at `commitment`, failing if storage lacks it.
    async fn get_block(&self, commitment: OLBlockCommitment) -> OLMempoolResult<OLBlockV1> {
        self.try_get_block(commitment).await?.ok_or_else(|| {
            OLMempoolError::AccountStateAccess(format!("Block not found for tip {commitment:?}"))
        })
    }

    /// Fetches the block at `commitment`, if storage has it.
    async fn try_get_block(
        &self,
        commitment: OLBlockCommitment,
    ) -> OLMempoolResult<Option<OLBlockV1>> {
        self.ctx
            .storage
            .ol_block()
            .get_block_data_async(*commitment.blkid())
            .await
            .map_err(|e| {
                OLMempoolError::AccountStateAccess(format!(
                    "Failed to get block for tip {commitment:?}: {e}"
                ))
            })
    }

    /// Finds how the chain moved from the mempool tip to `new_tip`.
    ///
    /// Walks the new branch down to the mempool tip's slot. If it meets the
    /// mempool tip, the update extends the chain. Otherwise it walks both
    /// branches down to their common ancestor, reverting at most
    /// [`OLMempoolConfig::max_reorg_depth`] blocks of the old branch. OL slots
    /// are contiguous, so each step lowers the slot by one. A deeper ancestor,
    /// or a branch block storage lacks, leaves the change unresolved.
    ///
    /// # Errors
    ///
    /// Returns an error if storage lacks a block between the mempool tip's
    /// slot and `new_tip`, as a forward walk always has.
    async fn find_tip_change(&self, new_tip: OLBlockCommitment) -> OLMempoolResult<TipChange> {
        let old_tip = self.tip.block();
        let mut adopted = Vec::new();
        let mut new_walk = new_tip;
        while new_walk.slot() > old_tip.slot() {
            adopted.push(new_walk);
            new_walk = parent_commitment(new_walk, &self.get_block(new_walk).await?);
        }
        if new_walk == old_tip {
            adopted.reverse();
            return Ok(TipChange::Extend { adopted });
        }

        // The new tip does not descend from the old one. The old walk stays at
        // or above the new walk's slot, and both step down while they share it.
        let max_depth = self.ctx.config.max_reorg_depth;
        let mut reverted = Vec::new();
        let mut old_walk = old_tip;
        while old_walk != new_walk {
            if reverted.len() as u64 >= max_depth {
                return Ok(TipChange::Unresolved { missing: None });
            }
            let same_slot = old_walk.slot() == new_walk.slot();
            let Some(old_block) = self.walkable_block(old_walk).await? else {
                return Ok(TipChange::Unresolved {
                    missing: Some(old_walk),
                });
            };
            let old_parent = parent_commitment(old_walk, &old_block);
            reverted.push(old_block);
            if same_slot {
                let Some(new_block) = self.walkable_block(new_walk).await? else {
                    return Ok(TipChange::Unresolved {
                        missing: Some(new_walk),
                    });
                };
                adopted.push(new_walk);
                new_walk = parent_commitment(new_walk, &new_block);
            }
            old_walk = old_parent;
        }

        reverted.reverse();
        adopted.reverse();
        Ok(TipChange::Reorg { reverted, adopted })
    }

    /// Fetches the block at `commitment` if a branch walk can step past it,
    /// which it cannot at genesis or when storage lacks the block.
    async fn walkable_block(
        &self,
        commitment: OLBlockCommitment,
    ) -> OLMempoolResult<Option<OLBlockV1>> {
        if commitment.slot() == 0 {
            return Ok(None);
        }
        self.try_get_block(commitment).await
    }

    /// Handle chain update.
    ///
    /// 1. Find the blocks the update reverts and adopts relative to the mempool tip
    /// 2. Load state for new tip and install it
    /// 3. Remove what the adopted blocks include
    /// 4. On a reorg, restore what the reverted blocks included
    /// 5. Readmit everything against the new state
    /// 6. Write the database changes and the new tip in one atomic batch, then commit
    ///
    /// Without a common ancestor, the mempool cannot tell which held
    /// transactions the new branch includes, so it evicts all of them and
    /// restores nothing.
    ///
    /// Steps 2 to 5 change only memory, and step 6 writes every database
    /// change they imply, all or nothing, before the update commits. The
    /// recorded tip lets a restart catch up on blocks the mempool never
    /// processed, as [`Self::load_from_db`] describes. If a step fails, memory
    /// goes back to the old tip, its state and what it held, and the mempool
    /// is out of sync: it serves no candidates and admits nothing, since it
    /// cannot tell which held transactions the new chain includes. The next
    /// update starts from the old tip again, so it retries the same removals
    /// and writes, and its success brings the mempool back in sync.
    ///
    /// # Errors
    ///
    /// Returns an error if a block, a state, or a mempool database operation
    /// fails.
    #[instrument(skip(self), fields(component = "ol_mempool", slot = new_tip.block().slot()))]
    pub(crate) async fn handle_chain_update(&mut self, new_tip: MempoolTip) -> OLMempoolResult<()> {
        let result = self.apply_chain_update(new_tip).await;
        match &result {
            Ok(()) => {
                if self.out_of_sync {
                    self.out_of_sync = false;
                    gauge!("strata_mempool_out_of_sync").set(0.0);
                    info!(tip = %self.tip.block(), "mempool back in sync with the chain");
                }
            }
            Err(error) => {
                counter!("strata_mempool_chain_update_failures_total").increment(1);
                if self.out_of_sync {
                    debug!(
                        %error,
                        mempool_tip = %self.tip.block(),
                        new_tip = %new_tip.block(),
                        "chain update failed again; the mempool stays out of sync"
                    );
                } else {
                    self.out_of_sync = true;
                    gauge!("strata_mempool_out_of_sync").set(1.0);
                    warn!(
                        %error,
                        mempool_tip = %self.tip.block(),
                        new_tip = %new_tip.block(),
                        "chain update failed; the mempool serves no candidates and admits \
                         nothing until an update succeeds"
                    );
                }
            }
        }
        result
    }

    /// Applies a chain update as [`Self::handle_chain_update`] describes,
    /// rolling memory back if it fails.
    async fn apply_chain_update(&mut self, new_tip: MempoolTip) -> OLMempoolResult<()> {
        debug!(
            old_tip = %self.tip.block(),
            new_tip = %new_tip.block(),
            "handling chain update"
        );
        let old_tip = self.tip.block();
        let tip_change = self.find_tip_change(new_tip.block()).await?;

        // Load the new tip's state and the spec of the block after it, and
        // install them together so admission never mixes two tips.
        let (new_state, tip_spec) = load_tip(&self.ctx, new_tip).await?;
        let held = self.install_tip(new_state, tip_spec);
        let outcome = match self.apply_tip_change(tip_change, new_tip.block()).await {
            Ok(outcome) => outcome,
            Err(e) => {
                self.roll_back(held);
                return Err(e);
            }
        };

        self.tip = new_tip;
        log_upgrade_required(
            new_tip.block(),
            held.tip_spec.upgrade_required(),
            &self.tip_spec,
        );
        match outcome {
            TipChangeOutcome::Unchanged | TipChangeOutcome::Extended => {}
            TipChangeOutcome::Reorged {
                reverted_blocks,
                adopted_blocks,
                restored,
            } => {
                counter!("strata_mempool_restored_total").increment(restored as u64);
                info!(
                    %reverted_blocks,
                    %adopted_blocks,
                    %restored,
                    "restored transactions from reverted blocks"
                );
            }
            TipChangeOutcome::Unresolved { missing, evicted } => {
                counter!("strata_mempool_reorg_evicted_total").increment(evicted as u64);
                warn!(
                    %old_tip,
                    new_tip = %new_tip.block(),
                    max_depth = %self.ctx.config.max_reorg_depth,
                    ?missing,
                    %evicted,
                    "no common ancestor with the new tip; evicted every held transaction"
                );
            }
        }
        Ok(())
    }

    /// Installs the new tip's state and spec and returns what memory held
    /// before, for [`Self::roll_back`].
    fn install_tip(&mut self, state: P::State, tip_spec: SelectedSpec) -> HeldState<P::State> {
        HeldState {
            entries: self.entries.clone(),
            account_state: self.account_state.clone(),
            stats: self.stats.clone(),
            state_accessor: mem::replace(&mut self.state_accessor, state),
            tip_spec: mem::replace(&mut self.tip_spec, tip_spec),
            deferred_load: self.deferred_load,
            needs_readmit: self.needs_readmit,
        }
    }

    /// Restores what memory held before a failed chain update.
    fn roll_back(&mut self, held: HeldState<P::State>) {
        self.entries = held.entries;
        self.account_state = held.account_state;
        self.stats = held.stats;
        self.state_accessor = held.state_accessor;
        self.tip_spec = held.tip_spec;
        self.deferred_load = held.deferred_load;
        self.needs_readmit = held.needs_readmit;
        self.record_mempool_gauges();
    }

    /// Applies `tip_change` to memory against the installed tip state, then
    /// writes the database changes it implies together with `new_tip`.
    async fn apply_tip_change(
        &mut self,
        tip_change: TipChange,
        new_tip: OLBlockCommitment,
    ) -> OLMempoolResult<TipChangeOutcome> {
        if self.deferred_load && matches!(self.tip_spec, SelectedSpec::Supported(_)) {
            self.load_stored()?;
        }

        let mut changes = StorageChanges::default();
        let outcome = match tip_change {
            // A repeated tip leaves the state, and so every admission, as it
            // was, unless memory holds stored transactions nothing validated.
            TipChange::Extend { adopted } if adopted.is_empty() && !self.needs_readmit => {
                TipChangeOutcome::Unchanged
            }
            TipChange::Extend { adopted } => {
                self.remove_included(&adopted, &mut changes).await?;
                self.readmit_against_tip(Vec::new(), &mut changes);
                TipChangeOutcome::Extended
            }
            TipChange::Reorg { reverted, adopted } => {
                let adopted_txids = self.remove_included(&adopted, &mut changes).await?;
                let restored = self.collect_restored(&reverted, &adopted_txids);
                let restored = self.readmit_against_tip(restored, &mut changes);
                TipChangeOutcome::Reorged {
                    reverted_blocks: reverted.len(),
                    adopted_blocks: adopted.len(),
                    restored,
                }
            }
            TipChange::Unresolved { missing } => TipChangeOutcome::Unresolved {
                missing,
                evicted: self.evict_all(&mut changes)?,
            },
        };
        self.apply_storage_changes(&changes, new_tip)?;
        Ok(outcome)
    }
}

/// Mempool database writes a chain update makes before it commits.
#[derive(Debug, Default)]
pub(crate) struct StorageChanges {
    /// Rows to delete: transactions the new chain includes or evicts.
    deletes: HashSet<OLTxId>,

    /// Restored transactions to store, whose entries memory holds.
    puts: Vec<OLTxId>,
}

/// What memory held before a chain update, to restore if it fails.
struct HeldState<S> {
    entries: HashMap<OLTxId, MempoolEntry>,
    account_state: HashMap<AccountId, AccountMempoolState>,
    stats: OLMempoolStats,
    state_accessor: S,
    tip_spec: SelectedSpec,
    deferred_load: bool,
    needs_readmit: bool,
}

/// What a committed chain update did, for its logs and metrics.
#[derive(Debug)]
enum TipChangeOutcome {
    Unchanged,
    Extended,
    Reorged {
        reverted_blocks: usize,
        adopted_blocks: usize,
        restored: usize,
    },
    Unresolved {
        missing: Option<OLBlockCommitment>,
        evicted: usize,
    },
}

/// How the chain moved from the mempool tip to a new tip.
#[derive(Debug)]
enum TipChange {
    /// The new tip descends from the mempool tip through `adopted`, oldest
    /// first. A repeated tip adopts nothing.
    Extend { adopted: Vec<OLBlockCommitment> },

    /// The new tip forks from the mempool tip. `reverted` holds the old
    /// branch's blocks above the common ancestor and `adopted` the new
    /// branch's, both oldest first. A revert to an ancestor adopts nothing.
    Reorg {
        reverted: Vec<OLBlockV1>,
        adopted: Vec<OLBlockCommitment>,
    },

    /// The common ancestor is deeper than the maximum reorg depth, or storage
    /// lacks the branch block `missing`. The walk reached only part of the new
    /// branch, so the mempool cannot tell which transactions it includes.
    Unresolved { missing: Option<OLBlockCommitment> },
}

/// Returns the parent of `block`, which `commitment` commits to.
fn parent_commitment(commitment: OLBlockCommitment, block: &OLBlockV1) -> OLBlockCommitment {
    OLBlockCommitment::new(commitment.slot() - 1, *block.header().parent_blkid())
}

/// Returns the account and sequence number of a snark account update.
fn snark_update_key(tx: &OLTransactionV1) -> Option<(AccountId, u64)> {
    match tx.payload() {
        TransactionPayloadV1::SnarkAccountUpdate(payload) => {
            Some((*payload.target(), payload.operation().update().seq_no()))
        }
        TransactionPayloadV1::GenericAccountMessage(_) => None,
    }
}

/// Returns the current Unix time in microseconds, which orders arrivals.
fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX epoch")
        .as_micros() as u64
}

/// Loads the state at `tip` and selects the spec of the block after it.
async fn load_tip<P: StateProvider>(
    ctx: &MempoolContext<P>,
    tip: MempoolTip,
) -> OLMempoolResult<(P::State, SelectedSpec)> {
    let block = tip.block();
    let state = ctx
        .provider
        .get_state_for_tip_async(block)
        .await
        .map_err(|e| {
            OLMempoolError::StateProvider(format!("Failed to load state for tip {block:?}: {e}"))
        })?
        .ok_or_else(|| {
            OLMempoolError::StateProvider(format!("State not found for tip {block:?}"))
        })?;
    let tip_spec = select_tip_spec(&ctx.storage, ctx.genesis_l1_block, tip, &state).await?;
    Ok((state, tip_spec))
}

/// Warns that admission stops when the block after `block` runs a spec this
/// binary does not implement, unless `prev`, the upgrade the previous tip ran
/// into, is the same one.
///
/// A stopped node keeps receiving chain updates for the same tip while its
/// last checkpoints finalize, so warning on every update would repeat one
/// condition.
fn log_upgrade_required(
    block: OLBlockCommitment,
    prev: Option<&UpgradeRequired>,
    tip_spec: &SelectedSpec,
) {
    if let SelectedSpec::Unsupported(upgrade) = tip_spec
        && prev != Some(upgrade)
    {
        warn!(
            %block,
            enactment_l1_height = upgrade.enactment_l1_height(),
            spec_version = upgrade.spec_version(),
            "the mempool admits no transactions until this node is upgraded"
        );
    }
}

impl<P: StateProvider> ServiceState for MempoolServiceState<P> {
    fn name(&self) -> &str {
        "mempool"
    }
}

/// Extracts transaction IDs grouped by account from a block.
///
/// Converts block transactions to mempool format (without accumulator proofs)
/// and computes txids that match the original mempool entries.
fn extract_account_txs_from_block(block: &OLBlockV1) -> HashMap<AccountId, Vec<OLTxId>> {
    let mut by_account: HashMap<AccountId, Vec<OLTxId>> = HashMap::new();

    if let Some(tx_segment) = block.body().tx_segment() {
        for tx in tx_segment.txs() {
            let mempool_tx = tx.clone().with_accumulator_proofs(None);
            let account = mempool_tx.target();
            let account = account.expect("all OL payload variants must have a target");
            let txid = mempool_tx.compute_txid();
            by_account.entry(account).or_default().push(txid);
        }
    }

    by_account
}

/// Returns true if a transaction reported as invalid should be removed from the mempool.
fn should_remove_tx(reason: MempoolTxInvalidReason) -> bool {
    matches!(reason, MempoolTxInvalidReason::Invalid)
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{BitcoinAmount, L1BlockRecord, MsgPayload};
    use strata_checkpoint_types::EpochSummary;
    use strata_identifiers::{BRIDGE_GATEWAY_ACCT_ID, Buf32, Buf64, EpochCommitment, Hash};
    use strata_ol_chain_types_v1::{
        BlockFlagsV1, OLBlockBodyV1, OLBlockHeaderV1, OLTxSegmentV1, SignedOLBlockHeaderV1,
    };
    use strata_ol_log_budget::TxLogBudgetError;
    use strata_ol_params::{BridgeParams, OLParams};
    use strata_ol_state_container::OLStateContainer;
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_state_types::{
        IAccountStateMut, ISnarkAccountStateMut, IStateAccessor, IStateAccessorMut, OLSpecVersions,
    };
    use strata_ol_stf::TxEffectsOverflow;
    use strata_ol_stf_v1::test_utils::{
        make_checkpoint_predicate_enactment_manifest, make_empty_manifest,
        make_op_return_bosd_descriptor, make_withdrawal_payload,
    };
    use strata_snark_acct_types::{
        LedgerRefs, OutputMessage, ProofState, Seqno, SnarkAccountUpdate, UpdateOperationData,
        UpdateOutputs,
    };

    use super::*;
    use crate::test_utils::{
        InMemoryStateProvider, MempoolDbFaults, create_faulty_mempool_db,
        create_test_account_id_with, create_test_block_commitment,
        create_test_constraints_with_slots, create_test_context,
        create_test_generic_tx_for_account, create_test_generic_tx_with_size,
        create_test_ol_state_for_tip, create_test_snark_tx_from_update,
        create_test_snark_tx_with_seq_no, create_test_snark_tx_with_seq_no_and_slots,
        create_test_state_provider, create_test_tx_with_id, snark_seq_no, tx_target, with_max_slot,
    };
    use crate::types::OLMempoolConfig;
    use crate::{DEFAULT_COMMAND_BUFFER_SIZE, DEFAULT_MAX_MEMPOOL_BYTES, DEFAULT_MAX_REORG_DEPTH};

    /// Builds an update from account 1 that sends `count` zero-value messages,
    /// which emit no logs, to account 2.
    fn create_test_snark_tx_with_messages(count: usize) -> OLTransactionV1 {
        let payload = MsgPayload::from_bytes(BitcoinAmount::try_from(0).unwrap(), vec![]).unwrap();
        let outputs = UpdateOutputs::new_empty().with_messages(vec![
            OutputMessage::new(
                create_test_account_id_with(2),
                payload
            );
            count
        ]);
        let operation = UpdateOperationData::new(
            0,
            ProofState::new(Buf32::zero(), 0),
            vec![],
            LedgerRefs::new_empty(),
            outputs,
            vec![],
        );
        create_test_snark_tx_from_update(
            create_test_account_id_with(1),
            SnarkAccountUpdate::new(operation, vec![]),
            create_test_constraints_with_slots(None, None),
        )
    }

    fn create_test_snark_tx_with_withdrawals(count: usize) -> OLTransactionV1 {
        let payload = MsgPayload::from_bytes(
            BitcoinAmount::try_from(BridgeParams::default().denomination()).unwrap(),
            make_withdrawal_payload(make_op_return_bosd_descriptor(0x42)),
        )
        .unwrap();
        let outputs = UpdateOutputs::new_empty().with_messages(vec![
            OutputMessage::new(
                BRIDGE_GATEWAY_ACCT_ID,
                payload
            );
            count
        ]);
        let operation = UpdateOperationData::new(
            0,
            ProofState::new(Buf32::zero(), 0),
            vec![],
            LedgerRefs::new_empty(),
            outputs,
            vec![],
        );
        create_test_snark_tx_from_update(
            create_test_account_id_with(1),
            SnarkAccountUpdate::new(operation, vec![]),
            create_test_constraints_with_slots(None, None),
        )
    }

    #[tokio::test]
    async fn test_oversized_log_replacement_preserves_pending_update_when_full() {
        let tip = create_test_block_commitment(100);
        let context = Arc::new(create_test_context(
            OLMempoolConfig {
                max_tx_count: 1,
                ..Default::default()
            },
            Arc::new(create_test_state_provider(tip)),
        ));
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();
        let original_id = state
            .add_transaction(create_test_snark_tx_with_withdrawals(172))
            .await
            .unwrap();

        let error = state
            .add_transaction(create_test_snark_tx_with_withdrawals(173))
            .await
            .expect_err("an update exceeding an empty checkpoint's log budget must be rejected");
        assert!(matches!(
            error,
            OLMempoolError::LogBudget(TxLogBudgetError::LogPayloadBytes { .. })
        ));

        let pending = state.handle_get_candidates().collect::<Vec<_>>();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].txid(), original_id);

        // Reload through the service path to verify the rejected replacement was not persisted.
        let mut reloaded =
            MempoolServiceState::new_with_context(state.ctx.clone(), MempoolTip::new(tip, None))
                .await
                .unwrap();
        reloaded.load_from_db().await.unwrap();
        let pending = reloaded.handle_get_candidates().collect::<Vec<_>>();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].txid(), original_id);
    }

    #[tokio::test]
    async fn test_log_budget_uses_configured_bridge_params() {
        let tip = create_test_block_commitment(100);
        let mut context = create_test_context(
            OLMempoolConfig::default(),
            Arc::new(create_test_state_provider(tip)),
        );
        // This network limbos the fixture's 81-byte descriptors, so they emit no logs.
        context.bridge_params = BridgeParams::new_with_descriptor_limit(
            BridgeParams::default().denomination(),
            None,
            21,
        )
        .unwrap();
        let mut state =
            MempoolServiceState::new_with_context(Arc::new(context), MempoolTip::new(tip, None))
                .await
                .unwrap();
        let txid = state
            .add_transaction(create_test_snark_tx_with_withdrawals(173))
            .await
            .unwrap();
        assert_eq!(state.handle_get_candidates().count(), 1);

        let mut reloaded =
            MempoolServiceState::new_with_context(state.ctx.clone(), MempoolTip::new(tip, None))
                .await
                .unwrap();
        reloaded.load_from_db().await.unwrap();
        let pending: Vec<_> = reloaded
            .handle_get_candidates()
            .map(|candidate| candidate.txid())
            .collect();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], txid);
    }

    #[tokio::test]
    async fn test_reload_removes_oversized_logs_and_successors() {
        let tip = create_test_block_commitment(100);
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            Arc::new(create_test_state_provider(tip)),
        ));
        let independent = create_test_snark_tx_with_seq_no(2, 0);
        let independent_id = independent.compute_txid();
        let message = create_test_generic_tx_for_account(1);
        let message_id = message.compute_txid();

        // Bypass submission to simulate transactions stored before budget enforcement.
        for (timestamp, tx) in [
            create_test_snark_tx_with_withdrawals(173),
            create_test_snark_tx_with_seq_no(1, 1),
            independent,
            create_test_snark_tx_with_seq_no(1, 2),
            message,
        ]
        .into_iter()
        .enumerate()
        {
            context
                .mempool_db
                .put_tx(MempoolTxData::new(
                    tx.compute_txid(),
                    tx.as_ssz_bytes(),
                    timestamp as u64,
                ))
                .unwrap();
        }

        let mut state =
            MempoolServiceState::new_with_context(context.clone(), MempoolTip::new(tip, None))
                .await
                .unwrap();
        state.load_from_db().await.unwrap();
        let pending: Vec<_> = state
            .handle_get_candidates()
            .map(|candidate| candidate.txid())
            .collect();
        assert_eq!(pending, vec![independent_id, message_id]);
        let persisted = context.mempool_db.get_all_txs().unwrap();
        assert_eq!(
            persisted
                .iter()
                .map(MempoolTxData::txid)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([independent_id, message_id])
        );
    }

    #[tokio::test]
    async fn test_oversized_logs_do_not_enter_empty_mempool() {
        let tip = create_test_block_commitment(100);
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            Arc::new(create_test_state_provider(tip)),
        ));
        let mut state =
            MempoolServiceState::new_with_context(context.clone(), MempoolTip::new(tip, None))
                .await
                .unwrap();
        for tx in [
            create_test_snark_tx_with_withdrawals(173),
            with_max_slot(
                create_test_snark_tx_with_withdrawals(173),
                Some(tip.slot() - 1),
            ),
        ] {
            assert!(matches!(
                state.add_transaction(tx).await,
                Err(OLMempoolError::LogBudget(
                    TxLogBudgetError::LogPayloadBytes { .. }
                ))
            ));
        }
        assert_eq!(state.handle_get_candidates().count(), 0);
        let mut reloaded =
            MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
                .await
                .unwrap();
        reloaded.load_from_db().await.unwrap();
        assert_eq!(reloaded.handle_get_candidates().count(), 0);
    }

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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let txs = state.handle_get_candidates().collect::<Vec<_>>();
        assert_eq!(txs.len(), 3);
        // All transactions target same account, should be in seq_no order
        let tx1_seq = snark_seq_no(txs[0].transaction()).expect("expected snark tx");
        let tx2_seq = snark_seq_no(txs[1].transaction()).expect("expected snark tx");
        let tx3_seq = snark_seq_no(txs[2].transaction()).expect("expected snark tx");
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();

        // Add three GAM transactions
        // They should get different priorities due to insertion order
        let gam1 = create_test_generic_tx_for_account(200);
        let gam1_target = tx_target(&gam1);
        state.add_transaction(gam1).await.unwrap();

        let gam2 = create_test_generic_tx_for_account(201);
        let gam2_target = tx_target(&gam2);
        state.add_transaction(gam2).await.unwrap();

        let gam3 = create_test_generic_tx_for_account(202);
        let gam3_target = tx_target(&gam3);
        state.add_transaction(gam3).await.unwrap();

        // All three GAM transactions
        // Should be ordered by insertion order (FIFO)
        let txs = state.handle_get_candidates().collect::<Vec<_>>();
        assert_eq!(txs.len(), 3);
        assert_eq!(tx_target(txs[0].transaction()), gam1_target); // First inserted
        assert_eq!(tx_target(txs[1].transaction()), gam2_target); // Second inserted
        assert_eq!(tx_target(txs[2].transaction()), gam3_target); // Third inserted
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();

        // Add three SnarkAccountUpdate transactions from DIFFERENT accounts
        // All with seq_no=0 (valid for each account)
        // They should get different priorities
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1_target = tx_target(&tx1);
        state.add_transaction(tx1).await.unwrap();

        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        let tx2_target = tx_target(&tx2);
        state.add_transaction(tx2).await.unwrap();

        let tx3 = create_test_snark_tx_with_seq_no(3, 0);
        let tx3_target = tx_target(&tx3);
        state.add_transaction(tx3).await.unwrap();

        // All three transactions have seq_no=0 but different accounts
        // Should be ordered by insertion order (FIFO)
        let txs = state.handle_get_candidates().collect::<Vec<_>>();
        assert_eq!(txs.len(), 3);
        assert_eq!(tx_target(txs[0].transaction()), tx1_target); // First inserted
        assert_eq!(tx_target(txs[1].transaction()), tx2_target); // Second inserted
        assert_eq!(tx_target(txs[2].transaction()), tx3_target); // Third inserted
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();

        // Create snark updates with sequential seq_nos: 0, 1, 2 for same account
        let snark1 = create_test_snark_tx_with_seq_no(1, 0);
        let snark1_target = tx_target(&snark1);

        let snark2 = create_test_snark_tx_with_seq_no(1, 1);
        let snark2_target = tx_target(&snark2);

        let snark3 = create_test_snark_tx_with_seq_no(1, 2);
        let snark3_target = tx_target(&snark3);

        // Add transactions with seq_no 0, 1, 2
        state.add_transaction(snark1).await.unwrap();
        state.add_transaction(snark2).await.unwrap();
        state.add_transaction(snark3).await.unwrap();

        // SnarkAccountUpdate transactions should be ordered by seq_no (0 < 1 < 2)
        let txs = state.handle_get_candidates().collect::<Vec<_>>();
        assert_eq!(txs.len(), 3);
        assert_eq!(tx_target(txs[0].transaction()), snark1_target); // seq_no 0
        assert_eq!(tx_target(txs[1].transaction()), snark2_target); // seq_no 1
        assert_eq!(tx_target(txs[2].transaction()), snark3_target); // seq_no 2
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state =
            MempoolServiceState::new_with_context(context.clone(), MempoolTip::new(tip, None))
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
        let mut state2 =
            MempoolServiceState::new_with_context(context.clone(), MempoolTip::new(tip, None))
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
    async fn test_handle_get_candidates_ordering() {
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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

        assert_eq!(
            state
                .handle_get_candidates()
                .map(|candidate| candidate.txid())
                .collect::<Vec<_>>(),
            vec![txid1, txid2, txid3, txid4]
        );
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();

        let initial_stats = state.stats();
        assert_eq!(initial_stats.mempool_size(), 0);
        assert_eq!(initial_stats.total_bytes(), 0);
        assert_eq!(initial_stats.enqueues_accepted(), 0);

        // Add first transaction - account 1 with seq_no 0
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1_size = Encode::as_ssz_bytes(&tx1).len();
        state.add_transaction(tx1.clone()).await.unwrap();

        let stats_after_first = state.stats();
        assert_eq!(stats_after_first.mempool_size(), 1);
        assert_eq!(stats_after_first.total_bytes(), tx1_size);
        assert_eq!(stats_after_first.enqueues_accepted(), 1);

        // Add second transaction - account 2 with seq_no 0
        let tx2 = create_test_snark_tx_with_seq_no(2, 0);
        let tx2_size = Encode::as_ssz_bytes(&tx2).len();
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state2 =
            MempoolServiceState::new_with_context(context_tiny, MempoolTip::new(tip2, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();

        // Add transactions with seq_no 0, 1, 2 where middle one expires first
        let mut tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let mut tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let mut tx2 = create_test_snark_tx_with_seq_no(1, 2);

        // Set max_slot so tx1 expires at slot 110, others at 120
        tx0 = with_max_slot(tx0, Some(120));
        tx1 = with_max_slot(tx1, Some(110));
        tx2 = with_max_slot(tx2, Some(120));

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

        // Readmit all transactions - expired tx1 is evicted
        readmit(&mut state);

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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
            .find(|(_, e)| snark_seq_no(&e.tx).is_some_and(|seq_no| seq_no == 2))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();

        // Use a known account ID that exists in test state (accounts 0-255 are created)
        let account_200 = create_test_account_id_with(200);

        // Create a transaction that exceeds the size limit
        let oversized_tx = create_test_generic_tx_with_size(
            account_200,
            max_tx_size + 100,
            create_test_constraints_with_slots(None, None),
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
            create_test_constraints_with_slots(None, None),
        );

        let result = state.add_transaction(valid_tx).await;
        assert!(result.is_ok());
        assert_eq!(state.stats().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_snapshot_retains_removed_bodies_and_excludes_later_arrivals() {
        let tip = create_test_block_commitment(100);
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            Arc::new(create_test_state_provider(tip)),
        ));
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
            .await
            .unwrap();
        let original = create_test_snark_tx_with_seq_no(1, 0);
        let original_id = state.add_transaction(original.clone()).await.unwrap();
        let snapshot = state.handle_get_candidates();

        state.handle_report_invalid_transactions(vec![(
            original_id,
            MempoolTxInvalidReason::Invalid,
        )]);
        let later_id = state
            .add_transaction(create_test_snark_tx_with_seq_no(2, 0))
            .await
            .unwrap();

        let captured = snapshot.collect::<Vec<_>>();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].txid(), original_id);
        assert_eq!(captured[0].transaction(), &original);
        assert_eq!(
            state
                .handle_get_candidates()
                .map(|candidate| candidate.txid())
                .collect::<Vec<_>>(),
            vec![later_id]
        );
    }

    #[tokio::test]
    async fn test_candidate_order_survives_replacement_and_reload() {
        let tip = create_test_block_commitment(100);
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            Arc::new(create_test_state_provider(tip)),
        ));
        let mut state =
            MempoolServiceState::new_with_context(context.clone(), MempoolTip::new(tip, None))
                .await
                .unwrap();
        let original = state
            .add_transaction(create_test_snark_tx_with_seq_no(1, 0))
            .await
            .unwrap();
        let successor = state
            .add_transaction(create_test_snark_tx_with_seq_no(1, 1))
            .await
            .unwrap();
        let independent = state
            .add_transaction(create_test_snark_tx_with_seq_no(2, 0))
            .await
            .unwrap();
        let mut snapshot = state.handle_get_candidates();
        let replacement = state
            .add_transaction(create_test_snark_tx_with_seq_no(1, 0))
            .await
            .unwrap();
        let original_candidate = snapshot.next().unwrap();
        assert_eq!(original_candidate.txid(), original);
        assert_eq!(original_candidate.transaction().compute_txid(), original);
        assert!(!state.contains(&original));
        let expected = vec![independent, replacement, successor];
        assert_eq!(
            state
                .handle_get_candidates()
                .map(|candidate| candidate.txid())
                .collect::<Vec<_>>(),
            expected
        );

        let mut reloaded =
            MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
                .await
                .unwrap();
        reloaded.load_from_db().await.unwrap();
        assert_eq!(
            reloaded
                .handle_get_candidates()
                .map(|candidate| candidate.txid())
                .collect::<Vec<_>>(),
            expected
        );
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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
        let mut state = MempoolServiceState::new_with_context(context, MempoolTip::new(tip, None))
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

    /// Stores the chain data of a tip at slot 100 that ends epoch 1 under
    /// `spec`, which processed the L1 block after the genesis anchor, and
    /// returns the tip and its state. With `enactment`, that block's manifest
    /// carries a checkpoint predicate enactment.
    async fn store_terminal_tip(
        storage: &NodeStorage,
        genesis_l1_block: L1BlockCommitment,
        spec: OLSpecId,
        enactment: bool,
    ) -> (MempoolTip, OLStateContainer) {
        let tip = store_child_block(storage, create_test_block_commitment(99), 1, &[]).await;
        let mut state =
            MemoryStateBaseLayer::from_container(create_test_ol_state_for_tip(tip.slot()));
        state.set_spec_versions(OLSpecVersions::uniform(spec));
        let anchor = L1BlockCommitment::new(state.last_l1_height(), *state.last_l1_blkid());
        assert_eq!(anchor, genesis_l1_block);
        let height = anchor.height() + 1;
        let manifest = if enactment {
            make_checkpoint_predicate_enactment_manifest(height, 1)
        } else {
            make_empty_manifest(height, 0)
        };
        state.append_l1_block_rec(
            height,
            L1BlockRecord::new(*manifest.blkid().as_ref(), [0; 32]),
        );
        let last_l1 = L1BlockCommitment::new(height, *manifest.blkid());

        storage.l1().put_block_data_async(manifest).await.unwrap();
        let genesis = create_test_block_commitment(0);
        let checkpoints = storage.ol_checkpoint();
        for summary in [
            EpochSummary::new(0, genesis, OLBlockCommitment::null(), anchor, Buf32::zero()),
            EpochSummary::new(1, tip, genesis, last_l1, Buf32::zero()),
        ] {
            checkpoints
                .insert_epoch_summary_async(summary)
                .await
                .unwrap();
        }
        (MempoolTip::new(tip, Some(1)), state.into_container())
    }

    /// Stores a mid-epoch block at slot 100 that is a sibling of the terminal
    /// tip [`store_terminal_tip`] stores, gives it a state, and returns it.
    /// Moving between the two is a one-block reorg.
    async fn store_mid_epoch_sibling(
        storage: &NodeStorage,
        provider: &InMemoryStateProvider,
    ) -> OLBlockCommitment {
        let tip = store_child_block(storage, create_test_block_commitment(99), 0, &[]).await;
        provider.insert_state(tip, create_test_ol_state_for_tip(tip.slot()));
        tip
    }

    /// Builds a mempool on the terminal tip [`store_terminal_tip`] stores, and
    /// returns it with its state provider.
    async fn mempool_on_terminal_tip(
        spec: OLSpecId,
        enactment: bool,
    ) -> (
        MempoolServiceState<InMemoryStateProvider>,
        Arc<InMemoryStateProvider>,
    ) {
        let mid_epoch_tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(mid_epoch_tip));
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            provider.clone(),
        ));
        let (tip, tip_state) =
            store_terminal_tip(&context.storage, context.genesis_l1_block, spec, enactment).await;
        provider.insert_state(tip.block(), tip_state);
        let state = MempoolServiceState::new_with_context(context, tip)
            .await
            .unwrap();
        (state, provider)
    }

    /// Readmits what `state` holds against its tip and writes the changes.
    fn readmit(state: &mut MempoolServiceState<InMemoryStateProvider>) {
        let mut changes = StorageChanges::default();
        state.readmit_against_tip(Vec::new(), &mut changes);
        state
            .apply_storage_changes(&changes, state.tip.block())
            .unwrap();
    }

    /// Stores `tx` in the mempool database, as an earlier run admitted it.
    fn store_tx(state: &MempoolServiceState<InMemoryStateProvider>, tx: &OLTransactionV1) {
        state
            .ctx
            .mempool_db
            .put_tx(MempoolTxData::new(tx.compute_txid(), tx.as_ssz_bytes(), 1))
            .unwrap();
    }

    /// After a terminal tip whose epoch processed a V2 enactment, the next
    /// block runs V2's successor. The mempool rejects new transactions and
    /// keeps the stored ones for later instead of dropping them.
    #[tokio::test]
    async fn test_mempool_admits_nothing_after_a_v2_enactment() {
        let (mut state, _) = mempool_on_terminal_tip(OLSpecId::V2, true).await;

        let err = state
            .add_transaction(create_test_snark_tx_with_seq_no(1, 0))
            .await
            .expect_err("no transaction is admitted");
        assert!(matches!(err, OLMempoolError::UpgradeRequired(_)), "{err}");

        let stored = create_test_snark_tx_with_seq_no(2, 0);
        store_tx(&state, &stored);
        state.load_from_db().await.unwrap();
        assert!(!state.contains(&stored.compute_txid()), "nothing is loaded");
        assert_eq!(state.ctx.mempool_db.get_all_txs().unwrap().len(), 1);
    }

    /// A transaction above the V2 effect capacities, admitted under V1, is
    /// evicted with its successors once the next block runs V2.
    #[tokio::test]
    async fn test_revalidation_evicts_effects_over_the_next_spec_capacity() {
        let max_messages = spec_limits(OLSpecId::V2).max_tx_messages();
        let (mut state, _) = mempool_on_terminal_tip(OLSpecId::V1, false).await;
        let over = state
            .add_transaction(create_test_snark_tx_with_messages(max_messages + 1))
            .await
            .expect("admitted under V1");
        let successor = state
            .add_transaction(create_test_snark_tx_with_seq_no(1, 1))
            .await
            .expect("successor admitted under V1");
        readmit(&mut state);
        assert!(state.contains(&over) && state.contains(&successor));

        state.tip_spec = SelectedSpec::Supported(OLSpecId::V2);
        readmit(&mut state);
        assert!(!state.contains(&over));
        assert!(!state.contains(&successor), "the successor is evicted too");
    }

    /// The mempool admits a transaction above the V2 effect capacities under
    /// V1 and rejects it under V2.
    #[tokio::test]
    async fn test_mempool_rejects_effects_over_the_spec_capacity() {
        let max_messages = spec_limits(OLSpecId::V2).max_tx_messages();
        let tx = create_test_snark_tx_with_messages(max_messages + 1);

        let (mut state, _) = mempool_on_terminal_tip(OLSpecId::V1, false).await;
        assert_eq!(state.select_spec().unwrap(), OLSpecId::V1);
        let txid = state
            .add_transaction(tx.clone())
            .await
            .expect("admitted under V1");
        assert!(state.contains(&txid));

        let (mut state, _) = mempool_on_terminal_tip(OLSpecId::V1, true).await;
        assert_eq!(state.select_spec().unwrap(), OLSpecId::V2);
        let err = state
            .add_transaction(tx)
            .await
            .expect_err("rejected under V2");
        assert!(
            matches!(
                err,
                OLMempoolError::EffectsOverflow(TxEffectsOverflow::Messages { count, max })
                    if count == max_messages + 1 && max == max_messages
            ),
            "{err}"
        );
        assert!(!state.contains(&txid));

        let txid = state
            .add_transaction(create_test_snark_tx_with_messages(max_messages))
            .await
            .expect("admitted at the V2 capacity");
        assert!(state.contains(&txid));
    }

    /// After a terminal tip whose epoch processed no enactment, the next block
    /// runs V1 and the mempool admits transactions.
    #[tokio::test]
    async fn test_mempool_admits_after_a_plain_epoch() {
        let (mut state, _) = mempool_on_terminal_tip(OLSpecId::V1, false).await;
        let txid = state
            .add_transaction(create_test_snark_tx_with_seq_no(1, 0))
            .await
            .expect("admitted under V1");
        assert!(state.contains(&txid));
    }

    /// After the last V0 terminal, which processed the enactment that ends V0,
    /// the next block runs V1 and the mempool admits transactions under it.
    #[tokio::test]
    async fn test_mempool_admits_under_v1_after_the_v0_enactment() {
        let (mut state, _) = mempool_on_terminal_tip(OLSpecId::V0, true).await;
        assert_eq!(state.select_spec().unwrap(), OLSpecId::V1);
        let txid = state
            .add_transaction(create_test_snark_tx_with_seq_no(1, 0))
            .await
            .expect("admitted under V1");
        assert!(state.contains(&txid));
    }

    /// After the last V1 terminal, which processed the enactment that ends V1,
    /// the next block runs V2 and the mempool admits transactions under it.
    #[tokio::test]
    async fn test_mempool_admits_under_v2_after_the_v1_enactment() {
        let (mut state, _) = mempool_on_terminal_tip(OLSpecId::V1, true).await;
        assert_eq!(state.select_spec().unwrap(), OLSpecId::V2);
        let txid = state
            .add_transaction(create_test_snark_tx_with_seq_no(1, 0))
            .await
            .expect("admitted under V2");
        assert!(state.contains(&txid));
    }

    /// The mempool launches before FCM repairs a summary that a crash left
    /// missing. On a terminal tip without its summary, it still selects the
    /// next block's spec, from the canonical summary of the epoch before,
    /// instead of failing to launch.
    #[tokio::test]
    async fn test_mempool_launches_on_a_terminal_tip_without_its_summary() {
        let mid_epoch_tip = create_test_block_commitment(100);
        let provider = Arc::new(create_test_state_provider(mid_epoch_tip));
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            provider.clone(),
        ));
        let (tip, tip_state) = store_terminal_tip(
            &context.storage,
            context.genesis_l1_block,
            OLSpecId::V2,
            true,
        )
        .await;
        provider.insert_state(tip.block(), tip_state);
        let tip_epoch = EpochCommitment::from_terminal(1, tip.block());
        assert!(
            context
                .storage
                .ol_checkpoint()
                .del_epoch_summary_async(tip_epoch)
                .await
                .unwrap()
        );

        let mut state = MempoolServiceState::new_with_context(context, tip)
            .await
            .expect("the mempool launches");
        let err = state
            .add_transaction(create_test_snark_tx_with_seq_no(1, 0))
            .await
            .expect_err("no transaction is admitted");
        assert!(matches!(err, OLMempoolError::UpgradeRequired(_)), "{err}");
    }

    /// When the tip moves to a terminal block whose epoch processed a V2
    /// enactment, the mempool keeps what it holds, revalidates nothing, and
    /// rejects even a resubmission. A reorg back to a block whose next block
    /// runs V1 resumes admission.
    #[tokio::test]
    async fn test_mempool_follows_the_tip_into_and_out_of_an_upgrade() {
        let provider = Arc::new(create_test_state_provider(create_test_block_commitment(
            100,
        )));
        let context = Arc::new(create_test_context(
            OLMempoolConfig::default(),
            provider.clone(),
        ));
        let mid_epoch_tip = store_mid_epoch_sibling(&context.storage, &provider).await;
        let mut state = MempoolServiceState::new_with_context(
            context.clone(),
            MempoolTip::new(mid_epoch_tip, None),
        )
        .await
        .unwrap();
        let held = create_test_snark_tx_with_seq_no(1, 0);
        let held_txid = state.add_transaction(held.clone()).await.unwrap();

        let (terminal_tip, terminal_state) = store_terminal_tip(
            &context.storage,
            context.genesis_l1_block,
            OLSpecId::V2,
            true,
        )
        .await;
        provider.insert_state(terminal_tip.block(), terminal_state);
        state.handle_chain_update(terminal_tip).await.unwrap();
        assert!(state.contains(&held_txid), "held transactions stay");
        let err = state
            .add_transaction(held)
            .await
            .expect_err("a resubmission is rejected too");
        assert!(matches!(err, OLMempoolError::UpgradeRequired(_)), "{err}");

        state
            .handle_chain_update(MempoolTip::new(mid_epoch_tip, None))
            .await
            .unwrap();
        state
            .add_transaction(create_test_snark_tx_with_seq_no(2, 0))
            .await
            .expect("admission resumes under V1");
        assert!(state.contains(&held_txid));
    }

    /// A mempool that starts while the next block needs an upgrade loads its
    /// stored transactions once a tip's next block runs V1 again.
    #[tokio::test]
    async fn test_mempool_loads_deferred_transactions_when_admission_resumes() {
        let (mut state, provider) = mempool_on_terminal_tip(OLSpecId::V2, true).await;
        let stored = create_test_snark_tx_with_seq_no(1, 0);
        store_tx(&state, &stored);
        state.load_from_db().await.unwrap();
        assert!(!state.contains(&stored.compute_txid()));

        let mid_epoch_tip = store_mid_epoch_sibling(&state.ctx.storage, &provider).await;
        state
            .handle_chain_update(MempoolTip::new(mid_epoch_tip, None))
            .await
            .unwrap();
        assert!(
            state.contains(&stored.compute_txid()),
            "loaded once admission resumes"
        );
    }

    /// A mempool on a stored base block at slot 10, with a state provider and
    /// storage to build branches on.
    struct ReorgFixture {
        state: MempoolServiceState<InMemoryStateProvider>,
        provider: Arc<InMemoryStateProvider>,
        storage: Arc<NodeStorage>,
        base: OLBlockCommitment,
        /// Makes mempool database writes fail while on.
        faults: Arc<MempoolDbFaults>,
    }

    impl ReorgFixture {
        async fn new(max_reorg_depth: u64) -> Self {
            Self::with_config(OLMempoolConfig {
                max_reorg_depth,
                ..OLMempoolConfig::default()
            })
            .await
        }

        async fn with_config(config: OLMempoolConfig) -> Self {
            let root = create_test_block_commitment(9);
            let probe = Arc::new(create_test_state_provider(root));
            let storage = create_test_context(config.clone(), probe).storage;
            let base = store_child_block(&storage, root, 0, &[]).await;
            let provider = Arc::new(InMemoryStateProvider::from_initial_state(
                base,
                reorg_state_at(base, &[]),
            ));
            let mut context = MempoolContext::new(
                config,
                BridgeParams::default(),
                storage.clone(),
                provider.clone(),
                OLParams::test_default().genesis_l1_block(),
            );
            let (mempool_db, faults) = create_faulty_mempool_db();
            context.mempool_db = mempool_db;
            let context = Arc::new(context);
            let mut state =
                MempoolServiceState::new_with_context(context, MempoolTip::new(base, None))
                    .await
                    .unwrap();
            state.load_from_db().await.unwrap();
            Self {
                state,
                provider,
                storage,
                base,
                faults,
            }
        }

        /// Stores a block on top of `parent` that includes `txs`.
        async fn block(
            &self,
            parent: OLBlockCommitment,
            salt: u64,
            txs: &[&OLTransactionV1],
        ) -> OLBlockCommitment {
            let txs: Vec<_> = txs.iter().map(|&tx| tx.clone()).collect();
            store_child_block(&self.storage, parent, salt, &txs).await
        }

        /// Moves the mempool to `tip`, whose state carries `seq_nos`.
        async fn move_tip(&mut self, tip: OLBlockCommitment, seq_nos: &[(u8, u64)]) {
            self.try_move_tip(tip, seq_nos).await.unwrap();
        }

        async fn try_move_tip(
            &mut self,
            tip: OLBlockCommitment,
            seq_nos: &[(u8, u64)],
        ) -> OLMempoolResult<()> {
            self.provider
                .insert_state(tip, reorg_state_at(tip, seq_nos));
            self.state
                .handle_chain_update(MempoolTip::new(tip, None))
                .await
        }

        /// Restarts the mempool at `tip`, whose state carries `seq_nos`, over
        /// the same storage, as the node does after a stop.
        async fn restart_at(&mut self, tip: OLBlockCommitment, seq_nos: &[(u8, u64)]) {
            self.provider
                .insert_state(tip, reorg_state_at(tip, seq_nos));
            self.state = MempoolServiceState::new_with_context(
                self.state.ctx.clone(),
                MempoolTip::new(tip, None),
            )
            .await
            .unwrap();
            self.state.load_from_db().await.unwrap();
        }

        fn recorded_tip(&self) -> Option<OLBlockCommitment> {
            self.state.ctx.mempool_db.get_tip().unwrap()
        }

        /// Asserts the mempool is out of sync: it serves no candidates and
        /// rejects a submission.
        async fn assert_out_of_sync(&mut self) {
            assert_eq!(self.candidate_txids(), Vec::<OLTxId>::new());
            let err = self
                .state
                .add_transaction(create_test_snark_tx_with_seq_no(9, 0))
                .await
                .expect_err("rejected while out of sync");
            assert!(matches!(err, OLMempoolError::OutOfSync), "{err}");
        }

        async fn add(&mut self, tx: &OLTransactionV1) -> OLTxId {
            self.state.add_transaction(tx.clone()).await.unwrap()
        }

        fn candidate_txids(&self) -> Vec<OLTxId> {
            self.state
                .handle_get_candidates()
                .map(|candidate| candidate.txid())
                .collect()
        }

        fn stored_txids(&self) -> HashSet<OLTxId> {
            self.state
                .ctx
                .mempool_db
                .get_all_txs()
                .unwrap()
                .iter()
                .map(|tx_data| tx_data.txid())
                .collect()
        }

        /// Asserts the mempool holds exactly `txs`, in memory and in storage.
        fn assert_holds(&self, txs: &[&OLTransactionV1]) {
            let expected: HashSet<OLTxId> = txs.iter().map(|tx| tx.compute_txid()).collect();
            let held: HashSet<OLTxId> = self.state.entries.keys().copied().collect();
            assert_eq!(held, expected, "in-memory transactions");
            assert_eq!(self.stored_txids(), expected, "stored transactions");
            assert_eq!(self.state.stats().mempool_size(), expected.len());
        }
    }

    /// Stores a block on top of `parent` that includes `txs`. `salt` tells
    /// sibling blocks apart.
    async fn store_child_block(
        storage: &NodeStorage,
        parent: OLBlockCommitment,
        salt: u64,
        txs: &[OLTransactionV1],
    ) -> OLBlockCommitment {
        let body = OLBlockBodyV1::new_common(OLTxSegmentV1::new(txs.to_vec()).unwrap());
        let header = OLBlockHeaderV1::new(
            salt,
            BlockFlagsV1::zero(),
            parent.slot() + 1,
            0,
            *parent.blkid(),
            body.compute_hash_commitment(),
            Buf32::zero(),
            Buf32::zero(),
        );
        let commitment = header.compute_block_commitment();
        let block = OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body);
        storage
            .ol_block()
            .put_block_data_async(block)
            .await
            .unwrap();
        commitment
    }

    /// Builds the state at `tip` where each listed account expects the given
    /// sequence number next and every other test account expects 0.
    fn reorg_state_at(tip: OLBlockCommitment, seq_nos: &[(u8, u64)]) -> OLStateContainer {
        let mut state =
            MemoryStateBaseLayer::from_container(create_test_ol_state_for_tip(tip.slot()));
        for &(account, seq_no) in seq_nos {
            state
                .update_account(create_test_account_id_with(account), |account| {
                    account.as_snark_account_mut().unwrap().set_proof_state(
                        Hash::zero(),
                        0,
                        Seqno::from(seq_no),
                    );
                })
                .unwrap();
        }
        state.into_container()
    }

    /// An extension removes what the new block includes, and a same-height
    /// reorg to a sibling that includes nothing restores it.
    #[tokio::test]
    async fn test_same_height_reorg_restores_reverted_transactions() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        f.add(&tx0).await;
        f.add(&tx1).await;

        let a1 = f.block(f.base, 1, &[&tx0, &tx1]).await;
        f.move_tip(a1, &[(1, 2)]).await;
        f.assert_holds(&[]);

        let b1 = f.block(f.base, 2, &[]).await;
        f.move_tip(b1, &[]).await;
        f.assert_holds(&[&tx0, &tx1]);
        assert_eq!(
            f.candidate_txids(),
            vec![tx0.compute_txid(), tx1.compute_txid()]
        );
    }

    /// Restored updates go back ahead of the successor the mempool admitted
    /// after the reverted block, which only stays valid behind them.
    #[tokio::test]
    async fn test_reorg_restores_reverted_updates_ahead_of_held_successors() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let other = create_test_generic_tx_for_account(2);
        let a1 = f.block(f.base, 1, &[&tx0, &tx1]).await;
        f.move_tip(a1, &[(1, 2)]).await;
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);
        f.add(&tx2).await;
        f.add(&other).await;

        let b1 = f.block(f.base, 2, &[]).await;
        f.move_tip(b1, &[]).await;
        f.assert_holds(&[&tx0, &tx1, &tx2, &other]);

        let account_1 = create_test_account_id_with(1);
        let account_1_order: Vec<OLTxId> = f
            .state
            .handle_get_candidates()
            .filter(|candidate| tx_target(candidate.transaction()) == account_1)
            .map(|candidate| candidate.txid())
            .collect();
        assert_eq!(
            account_1_order,
            vec![tx0.compute_txid(), tx1.compute_txid(), tx2.compute_txid()]
        );
        assert_eq!(
            f.state.account_state[&account_1].seq_no_range(),
            Some((0, 2))
        );
        assert_eq!(
            f.candidate_txids()[0],
            tx0.compute_txid(),
            "restored transactions arrived before held ones"
        );
    }

    /// A reorg to a branch shorter than the reverted one restores every
    /// reverted block's transactions.
    #[tokio::test]
    async fn test_shorter_tip_reorg_restores_reverted_transactions() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let a1 = f.block(f.base, 1, &[&tx0]).await;
        let a2 = f.block(a1, 1, &[&tx1]).await;
        f.move_tip(a2, &[(1, 2)]).await;
        f.assert_holds(&[]);

        let b1 = f.block(f.base, 2, &[]).await;
        f.move_tip(b1, &[]).await;
        f.assert_holds(&[&tx0, &tx1]);
    }

    /// A revert to an ancestor of the tip restores the reverted blocks'
    /// transactions without adopting any block.
    #[tokio::test]
    async fn test_revert_to_ancestor_restores_reverted_transactions() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let a1 = f.block(f.base, 1, &[&tx0]).await;
        let a2 = f.block(a1, 1, &[&tx1]).await;
        f.move_tip(a2, &[(1, 2)]).await;

        f.move_tip(f.base, &[]).await;
        f.assert_holds(&[&tx0, &tx1]);
    }

    /// A reorg that reverts several blocks for a longer branch restores all
    /// their transactions in dependency order.
    #[tokio::test]
    async fn test_deeper_reorg_restores_reverted_transactions() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let txs: Vec<_> = (0..3)
            .map(|seq_no| create_test_snark_tx_with_seq_no(1, seq_no))
            .collect();
        let message = create_test_generic_tx_for_account(3);
        let a1 = f.block(f.base, 1, &[&txs[0]]).await;
        let a2 = f.block(a1, 1, &[&txs[1], &message]).await;
        let a3 = f.block(a2, 1, &[&txs[2]]).await;
        f.move_tip(a3, &[(1, 3)]).await;
        f.assert_holds(&[]);

        let mut b = f.base;
        for _ in 0..4 {
            b = f.block(b, 2, &[]).await;
        }
        f.move_tip(b, &[]).await;
        f.assert_holds(&[&txs[0], &txs[1], &txs[2], &message]);
        let order: Vec<OLTxId> = f
            .candidate_txids()
            .into_iter()
            .filter(|txid| *txid != message.compute_txid())
            .collect();
        let expected: Vec<OLTxId> = txs.iter().map(OLTransactionV1::compute_txid).collect();
        assert_eq!(order, expected);
    }

    /// A reverted transaction the adopted branch includes too stays out of the
    /// mempool. Messages carry no sequence number, so only this check keeps
    /// one from being included twice.
    #[tokio::test]
    async fn test_reorg_does_not_restore_transactions_the_adopted_branch_includes() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let message = create_test_generic_tx_for_account(3);
        let update = create_test_snark_tx_with_seq_no(1, 0);
        let reverted_only = create_test_snark_tx_with_seq_no(2, 0);
        let a1 = f
            .block(f.base, 1, &[&message, &update, &reverted_only])
            .await;
        f.move_tip(a1, &[(1, 1), (2, 1)]).await;

        let b1 = f.block(f.base, 2, &[&message, &update]).await;
        let b2 = f.block(b1, 2, &[]).await;
        f.move_tip(b2, &[(1, 1)]).await;
        f.assert_holds(&[&reverted_only]);
    }

    /// A reverted transaction the new tip invalidates is dropped with its
    /// successors instead of being restored, and never reaches storage.
    #[tokio::test]
    async fn test_reorg_drops_reverted_transactions_invalid_on_the_new_tip() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let expiring = create_test_snark_tx_with_seq_no_and_slots(1, 0, None, Some(11));
        let successor = create_test_snark_tx_with_seq_no(1, 1);
        let valid = create_test_snark_tx_with_seq_no(2, 0);
        f.add(&expiring).await;
        f.add(&successor).await;
        f.add(&valid).await;
        let a1 = f.block(f.base, 1, &[&expiring, &successor, &valid]).await;
        f.move_tip(a1, &[(1, 2), (2, 1)]).await;
        f.assert_holds(&[]);

        // The new branch reaches slot 12, past the expiring update's max slot.
        let b1 = f.block(f.base, 2, &[]).await;
        let b2 = f.block(b1, 2, &[]).await;
        f.move_tip(b2, &[]).await;
        f.assert_holds(&[&valid]);
    }

    /// A reorg deeper than the maximum depth evicts every held transaction.
    /// The walk stops before the adopted block that includes a held message,
    /// which carries no sequence number, so keeping it could apply it twice.
    /// The mempool still follows the new branch afterwards.
    #[tokio::test]
    async fn test_reorg_beyond_max_depth_evicts_everything() {
        let mut f = ReorgFixture::new(1).await;
        let message = create_test_generic_tx_for_account(3);
        let update = create_test_snark_tx_with_seq_no(2, 0);
        f.add(&message).await;
        f.add(&update).await;
        let a1 = f.block(f.base, 1, &[]).await;
        let a2 = f.block(a1, 1, &[]).await;
        f.move_tip(a2, &[]).await;
        f.assert_holds(&[&message, &update]);

        // The walk visits b3 and b2, then stops at the depth limit before b1.
        let b1 = f.block(f.base, 2, &[&message]).await;
        let b2 = f.block(b1, 2, &[]).await;
        let b3 = f.block(b2, 2, &[]).await;
        f.move_tip(b3, &[]).await;
        f.assert_holds(&[]);
        assert_eq!(f.state.stats().reorg_evictions(), 2);

        let next = create_test_snark_tx_with_seq_no(4, 0);
        f.add(&next).await;
        let b4 = f.block(b3, 2, &[&next]).await;
        f.move_tip(b4, &[(4, 1)]).await;
        f.assert_holds(&[]);
    }

    /// A reorg whose old-branch block storage lacks evicts every held
    /// transaction, including stored ones a load has not read.
    #[tokio::test]
    async fn test_reorg_with_a_missing_block_evicts_everything() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let unstored_tip = create_test_block_commitment(11);
        f.provider
            .insert_state(unstored_tip, reorg_state_at(unstored_tip, &[]));
        f.state = MempoolServiceState::new_with_context(
            f.state.ctx.clone(),
            MempoolTip::new(unstored_tip, None),
        )
        .await
        .unwrap();
        let message = create_test_generic_tx_for_account(3);
        let update = create_test_snark_tx_with_seq_no(2, 0);
        f.add(&message).await;
        f.add(&update).await;
        let unloaded = create_test_snark_tx_with_seq_no(5, 0);
        store_tx(&f.state, &unloaded);

        let b1 = f.block(f.base, 2, &[]).await;
        f.move_tip(b1, &[]).await;
        f.assert_holds(&[]);
        assert_eq!(f.state.stats().reorg_evictions(), 3);
    }

    /// A one-block reorg is within a maximum depth of one and restores.
    #[tokio::test]
    async fn test_reorg_at_max_depth_restores() {
        let mut f = ReorgFixture::new(1).await;
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let a1 = f.block(f.base, 1, &[&tx0]).await;
        f.move_tip(a1, &[(1, 1)]).await;

        let b1 = f.block(f.base, 2, &[]).await;
        let b2 = f.block(b1, 2, &[]).await;
        f.move_tip(b2, &[]).await;
        f.assert_holds(&[&tx0]);
    }

    /// Restored transactions count against the transaction limit. They
    /// arrived before the held ones, so a full mempool keeps them and evicts
    /// its most recently arrived held transactions.
    #[tokio::test]
    async fn test_reorg_restores_within_the_count_limit_by_arrival() {
        let mut f = ReorgFixture::with_config(OLMempoolConfig {
            max_tx_count: 3,
            ..OLMempoolConfig::default()
        })
        .await;
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let a1 = f.block(f.base, 1, &[&tx0, &tx1]).await;
        f.move_tip(a1, &[(1, 2)]).await;
        let older = create_test_generic_tx_for_account(2);
        let newer = create_test_generic_tx_for_account(3);
        f.add(&older).await;
        f.add(&newer).await;

        let b1 = f.block(f.base, 2, &[]).await;
        f.move_tip(b1, &[]).await;
        f.assert_holds(&[&tx0, &tx1, &older]);
        assert_eq!(f.state.stats().evictions(), 1);
    }

    /// Restored transactions count against the byte limit, and a held one
    /// that no longer fits is evicted.
    #[tokio::test]
    async fn test_reorg_restores_within_the_byte_limit() {
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let held = create_test_snark_tx_with_seq_no(2, 0);
        let mut f = ReorgFixture::with_config(OLMempoolConfig {
            max_mempool_bytes: tx0.ssz_bytes_len() + held.ssz_bytes_len() - 1,
            ..OLMempoolConfig::default()
        })
        .await;
        let a1 = f.block(f.base, 1, &[&tx0]).await;
        f.move_tip(a1, &[(1, 1)]).await;
        f.add(&held).await;

        let b1 = f.block(f.base, 2, &[]).await;
        f.move_tip(b1, &[]).await;
        f.assert_holds(&[&tx0]);
        assert_eq!(f.state.stats().evictions(), 1);
    }

    /// When the reverted transactions alone exceed the limit, the mempool
    /// restores them in chain order until it is full.
    #[tokio::test]
    async fn test_reorg_restores_reverted_transactions_up_to_the_limit() {
        let mut f = ReorgFixture::with_config(OLMempoolConfig {
            max_tx_count: 2,
            ..OLMempoolConfig::default()
        })
        .await;
        let first = create_test_snark_tx_with_seq_no(1, 0);
        let second = create_test_snark_tx_with_seq_no(2, 0);
        let third = create_test_snark_tx_with_seq_no(3, 0);
        let a1 = f.block(f.base, 1, &[&first, &second]).await;
        let a2 = f.block(a1, 1, &[&third]).await;
        f.move_tip(a2, &[(1, 1), (2, 1), (3, 1)]).await;

        f.move_tip(f.base, &[]).await;
        f.assert_holds(&[&first, &second]);
        assert_eq!(f.state.stats().evictions(), 1);
    }

    /// An extension checks each account's first held update against the
    /// chain. A block that includes a conflicting update for a held sequence
    /// number evicts the held one, and the held successor stays valid on top
    /// of the chain.
    #[tokio::test]
    async fn test_extension_evicts_held_updates_the_chain_supersedes() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let held0 = create_test_snark_tx_with_seq_no(1, 0);
        let held1 = create_test_snark_tx_with_seq_no(1, 1);
        f.add(&held0).await;
        f.add(&held1).await;

        let conflicting = create_test_snark_tx_with_seq_no(1, 0);
        assert_ne!(conflicting.compute_txid(), held0.compute_txid());
        let a1 = f.block(f.base, 1, &[&conflicting]).await;
        f.move_tip(a1, &[(1, 1)]).await;
        f.assert_holds(&[&held1]);
    }

    /// An extension that moves an account past every held update evicts them
    /// all, while other accounts' transactions stay.
    #[tokio::test]
    async fn test_extension_evicts_held_updates_behind_the_chain() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let held0 = create_test_snark_tx_with_seq_no(1, 0);
        let held1 = create_test_snark_tx_with_seq_no(1, 1);
        let other = create_test_snark_tx_with_seq_no(2, 0);
        f.add(&held0).await;
        f.add(&held1).await;
        f.add(&other).await;

        let conflicting0 = create_test_snark_tx_with_seq_no(1, 0);
        let conflicting1 = create_test_snark_tx_with_seq_no(1, 1);
        let a1 = f.block(f.base, 1, &[&conflicting0]).await;
        let a2 = f.block(a1, 1, &[&conflicting1]).await;
        f.move_tip(a2, &[(1, 2)]).await;
        f.assert_holds(&[&other]);
    }

    /// A failed delete during an unresolved reorg keeps the old tip and every
    /// held transaction, in memory and in storage, so a reload still sees the
    /// rows and nothing counts as evicted. The mempool is out of sync until
    /// the next chain update evicts everything.
    #[tokio::test]
    async fn test_unresolved_reorg_keeps_the_tip_when_a_delete_fails() {
        let mut f = ReorgFixture::new(1).await;
        let message = create_test_generic_tx_for_account(3);
        let update = create_test_snark_tx_with_seq_no(2, 0);
        f.add(&message).await;
        f.add(&update).await;
        let a1 = f.block(f.base, 1, &[]).await;
        let a2 = f.block(a1, 1, &[]).await;
        f.move_tip(a2, &[]).await;
        let b1 = f.block(f.base, 2, &[&message]).await;
        let b2 = f.block(b1, 2, &[]).await;
        let b3 = f.block(b2, 2, &[]).await;

        f.faults.set_fail_deletes(true);
        let err = f.try_move_tip(b3, &[]).await.expect_err("the delete fails");
        assert!(matches!(err, OLMempoolError::Database(_)), "{err}");
        assert_eq!(f.state.tip.block(), a2, "the tip stays");
        f.assert_holds(&[&message, &update]);
        assert_eq!(f.state.stats().reorg_evictions(), 0);
        f.assert_out_of_sync().await;

        f.faults.set_fail_deletes(false);
        f.move_tip(b3, &[]).await;
        assert_eq!(f.state.tip.block(), b3);
        f.assert_holds(&[]);
        assert_eq!(f.state.stats().reorg_evictions(), 2);
        assert!(!f.state.out_of_sync);
    }

    /// A failed delete of a block-included message leaves the old tip, its
    /// state, and everything held as they were, and the mempool out of sync:
    /// it serves no candidates, so the included message cannot be proposed
    /// again, and rejects submissions. The next chain update removes the
    /// message and brings the mempool back.
    #[tokio::test]
    async fn test_extension_keeps_the_tip_when_an_included_delete_fails() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let message = create_test_generic_tx_for_account(3);
        let held = create_test_snark_tx_with_seq_no(2, 0);
        f.add(&message).await;
        f.add(&held).await;
        let a1 = f.block(f.base, 1, &[&message]).await;

        f.faults.set_fail_deletes(true);
        let err = f.try_move_tip(a1, &[]).await.expect_err("the delete fails");
        assert!(matches!(err, OLMempoolError::Database(_)), "{err}");
        assert_eq!(f.state.tip.block(), f.base, "the tip stays");
        assert_eq!(
            f.state.state_accessor.cur_slot(),
            f.base.slot(),
            "the state stays"
        );
        f.assert_holds(&[&message, &held]);
        f.assert_out_of_sync().await;

        f.faults.set_fail_deletes(false);
        f.move_tip(a1, &[]).await;
        assert_eq!(f.state.tip.block(), a1);
        f.assert_holds(&[&held]);
        assert_eq!(f.candidate_txids(), vec![held.compute_txid()]);
        let next = create_test_snark_tx_with_seq_no(4, 0);
        f.add(&next).await;
        f.assert_holds(&[&held, &next]);
    }

    /// A reorg's database writes land in one atomic batch. When the second
    /// restored transaction cannot be stored, the first is not stored either,
    /// and the recorded tip stays, so nothing the old tip's chain includes is
    /// left behind for a restart to reload. The mempool is out of sync at the
    /// old tip, and the next chain update stores both.
    #[tokio::test]
    async fn test_reorg_writes_nothing_when_a_restored_write_fails() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let message = create_test_generic_tx_for_account(3);
        let update = create_test_snark_tx_with_seq_no(1, 0);
        let a1 = f.block(f.base, 1, &[&message, &update]).await;
        f.move_tip(a1, &[(1, 1)]).await;
        assert_eq!(f.recorded_tip(), Some(a1));
        let b1 = f.block(f.base, 2, &[]).await;

        // One restored write succeeds and the next fails, and so would any
        // delete that tried to undo the first.
        f.faults.set_put_budget(1);
        f.faults.set_fail_deletes(true);
        let err = f.try_move_tip(b1, &[]).await.expect_err("the write fails");
        assert!(matches!(err, OLMempoolError::Database(_)), "{err}");
        assert_eq!(f.state.tip.block(), a1, "the tip stays");
        f.assert_holds(&[]);
        assert_eq!(f.recorded_tip(), Some(a1));
        assert_eq!(f.state.stats().enqueues_accepted(), 0);
        f.assert_out_of_sync().await;

        f.faults.set_fail_puts(false);
        f.faults.set_fail_deletes(false);
        f.move_tip(b1, &[]).await;
        assert_eq!(f.state.tip.block(), b1);
        f.assert_holds(&[&message, &update]);
        assert_eq!(f.recorded_tip(), Some(b1));
        assert!(!f.state.out_of_sync);
    }

    /// A node that stops after a block lands but before the mempool commits
    /// the chain update for it restarts with that block's transactions still
    /// stored. Loading catches up from the recorded tip and removes the
    /// included generic message before the mempool serves candidates.
    #[tokio::test]
    async fn test_restart_catches_up_on_blocks_the_mempool_missed() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let message = create_test_generic_tx_for_account(3);
        let held = create_test_snark_tx_with_seq_no(2, 0);
        f.add(&message).await;
        f.add(&held).await;
        assert_eq!(f.recorded_tip(), Some(f.base));

        // a1 includes the message, and the node stops before the mempool
        // processes it, then restarts at a1.
        let a1 = f.block(f.base, 1, &[&message]).await;
        f.restart_at(a1, &[]).await;
        assert_eq!(f.state.tip.block(), a1);
        assert_eq!(f.candidate_txids(), vec![held.compute_txid()]);
        f.assert_holds(&[&held]);
        assert_eq!(f.recorded_tip(), Some(a1));
    }

    /// A restart that coincides with a reorg: the recorded branch included
    /// seq 0, the database holds its successor seq 1, and the launch branch
    /// reverted seq 0. Loading validates nothing before the catch-up restores
    /// seq 0, so seq 1 is not mistaken for a gap, and both are readmitted in
    /// order.
    #[tokio::test]
    async fn test_restart_during_a_reorg_keeps_stored_successors() {
        let mut f = ReorgFixture::new(DEFAULT_MAX_REORG_DEPTH).await;
        let seq0 = create_test_snark_tx_with_seq_no(1, 0);
        let seq1 = create_test_snark_tx_with_seq_no(1, 1);
        let a1 = f.block(f.base, 1, &[&seq0]).await;
        f.move_tip(a1, &[(1, 1)]).await;
        f.add(&seq1).await;
        assert_eq!(f.recorded_tip(), Some(a1));

        // Fork choice moves to b1, which reverts a1, and the node stops before
        // the mempool processes it, then restarts at b1.
        let b1 = f.block(f.base, 2, &[]).await;
        f.restart_at(b1, &[]).await;
        assert_eq!(f.state.tip.block(), b1);
        f.assert_holds(&[&seq0, &seq1]);
        assert_eq!(
            f.candidate_txids(),
            vec![seq0.compute_txid(), seq1.compute_txid()]
        );
        assert_eq!(f.recorded_tip(), Some(b1));
    }
}
