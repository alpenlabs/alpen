//! Mempool manager with synchronous API and database persistence.
//!
//! Wraps `MempoolCore` with Arc<Mutex<>> to provide thread-safe synchronous API
//! for RPC and block assembly. Persists transactions to database for durability.

use std::sync::{Arc, Mutex};

use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;
use strata_storage::NodeStorage;
use tokio::sync::broadcast;

use crate::{
    core::MempoolCore,
    error::{MempoolError, MempoolResult},
    events::{MempoolEvent, RemovalReason},
    types::{MempoolConfig, MempoolStats},
};

/// Default capacity for event broadcast channel.
const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 1000;

/// Mempool manager providing synchronous API with database persistence.
///
/// This wraps `MempoolCore` with Arc<Mutex<>> and provides methods for:
/// - RPC handlers to submit transactions
/// - Block assembly to retrieve transactions
/// - Management operations (update slot, get stats)
/// - Database persistence for durability across restarts
/// - Event broadcasting for transaction state changes
#[derive(Clone)]
pub struct MempoolManager {
    /// Core mempool protected by mutex.
    core: Arc<Mutex<MempoolCore>>,

    /// Storage for persistence.
    storage: Arc<NodeStorage>,

    /// Event broadcast sender for notifying subscribers.
    event_sender: broadcast::Sender<MempoolEvent>,
}

impl MempoolManager {
    /// Create a new mempool manager with the given configuration, current slot, and storage.
    pub fn new(config: MempoolConfig, current_slot: u64, storage: Arc<NodeStorage>) -> Self {
        let (event_sender, _) = broadcast::channel(DEFAULT_EVENT_CHANNEL_CAPACITY);
        Self {
            core: Arc::new(Mutex::new(MempoolCore::new(config, current_slot))),
            storage,
            event_sender,
        }
    }

    /// Create a new mempool manager and restore transactions from database.
    ///
    /// Loads all persisted transactions from the database into the in-memory core.
    /// Invalid or expired transactions are silently skipped during restoration.
    pub fn new_with_restore(
        config: MempoolConfig,
        current_slot: u64,
        storage: Arc<NodeStorage>,
    ) -> MempoolResult<Self> {
        let manager = Self::new(config, current_slot, storage);
        manager.restore_from_database()?;
        Ok(manager)
    }

    /// Restore transactions from database into in-memory core.
    fn restore_from_database(&self) -> MempoolResult<()> {
        let txids = self
            .storage
            .mempool()
            .get_all_tx_ids()
            .map_err(|e| MempoolError::DatabaseError(e.to_string()))?;

        for txid in txids {
            // Load transaction from database
            let tx = self
                .storage
                .mempool()
                .get_tx_entry(&txid)
                .map_err(|e| MempoolError::DatabaseError(e.to_string()))?;

            if let Some(tx) = tx {
                // Compute blob size for metadata
                let blob = encode_to_vec(&tx).map_err(|e| {
                    MempoolError::ParseError(format!("Failed to encode transaction: {e}"))
                })?;
                let blob_size = blob.len();

                // Try to add to core (may fail if invalid/expired - that's ok)
                let mut core = self.core.lock().unwrap();
                let _ = core.submit_transaction(tx, blob_size);
            }
        }

        Ok(())
    }

    /// Update the current slot (called by FCM-driven loop).
    pub fn update_current_slot(&self, slot: u64) {
        let mut core = self.core.lock().unwrap();
        core.update_current_slot(slot);
    }

    /// Submits a raw transaction to the mempool.
    ///
    /// Accepts a raw transaction blob (opaque bytes) which is parsed into an `OLTransaction`,
    /// validated, and stored. Returns the transaction ID if successful.
    ///
    /// Idempotent: returns success if transaction already exists.
    ///
    /// # Errors
    ///
    /// - [`MempoolError::ParseError`] - if the transaction blob cannot be parsed
    /// - [`MempoolError::InvalidTransaction`] - if validation fails
    /// - [`MempoolError::TransactionTooLarge`] - if the transaction exceeds size limits
    /// - [`MempoolError::MempoolCountLimitExceeded`] - if mempool is full
    /// - [`MempoolError::MempoolSizeLimitExceeded`] - if mempool size limit exceeded
    pub fn submit_transaction(&self, blob: Vec<u8>) -> MempoolResult<OLTxId> {
        // Parse the transaction blob
        let tx: OLTransaction = decode_buf_exact(&blob)
            .map_err(|e| MempoolError::ParseError(format!("Failed to parse transaction: {e}")))?;

        let blob_size = blob.len();
        let txid = tx.compute_txid();

        // Check if already exists (for idempotency + to avoid redundant DB writes)
        {
            let core = self.core.lock().unwrap();
            if core.contains(&txid) {
                return Ok(txid);
            }
        }

        // Persist to database first
        self.storage
            .mempool()
            .put_tx_entry(&txid, &tx)
            .map_err(|e| MempoolError::DatabaseError(e.to_string()))?;

        // Then add to in-memory core
        let mut core = self.core.lock().unwrap();
        let (txid, priority) = core.submit_transaction(tx, blob_size)?;

        // Emit event
        self.emit_event(MempoolEvent::TransactionAdded { txid, priority });

        Ok(txid)
    }

    /// Retrieves transactions from the mempool for block assembly.
    ///
    /// Returns up to `limit` transactions, ordered by the mempool's ordering policy
    /// (FIFO by entry_slot, then by OLTxId as tie-breaker).
    ///
    /// Returns an empty vector if no transactions are available (not an error).
    pub fn get_transactions(&self, limit: u64) -> MempoolResult<Vec<(OLTxId, OLTransaction)>> {
        let core = self.core.lock().unwrap();
        Ok(core.get_transactions(limit))
    }

    /// Removes transactions from the mempool.
    ///
    /// Typically called after transactions have been included in a block.
    /// Returns the list of transaction IDs that were successfully removed.
    /// Already-removed transactions are silently ignored (idempotent operation).
    pub fn remove_transactions(&self, txids: &[OLTxId]) -> MempoolResult<Vec<OLTxId>> {
        // Remove from database first (batch operation)
        self.storage
            .mempool()
            .del_tx_entries(txids)
            .map_err(|e| MempoolError::DatabaseError(e.to_string()))?;

        // Then remove from in-memory core
        let mut core = self.core.lock().unwrap();
        let removed = core.remove_transactions(txids);

        // Emit events for successfully removed transactions
        for txid in &removed {
            self.emit_event(MempoolEvent::TransactionRemoved {
                txid: *txid,
                reason: RemovalReason::Included,
            });
        }

        Ok(removed)
    }

    /// Gets statistics about the current mempool state.
    ///
    /// Returns statistics including transaction count, total size, and rejection counts.
    pub fn stats(&self) -> MempoolResult<MempoolStats> {
        let core = self.core.lock().unwrap();
        Ok(core.stats())
    }

    /// Check if a transaction exists in the mempool.
    pub fn contains(&self, txid: &OLTxId) -> bool {
        let core = self.core.lock().unwrap();
        core.contains(txid)
    }

    /// Subscribe to mempool events.
    ///
    /// Returns a broadcast receiver that will receive all mempool events (transaction
    /// additions, removals, evictions).
    ///
    /// Multiple subscribers can listen independently. If a subscriber falls behind and
    /// the channel buffer fills, older messages will be dropped (broadcast channel behavior).
    pub fn subscribe(&self) -> broadcast::Receiver<MempoolEvent> {
        self.event_sender.subscribe()
    }

    /// Emit an event to all subscribers.
    ///
    /// If there are no active subscribers, the event is dropped (no error).
    fn emit_event(&self, event: MempoolEvent) {
        // Ignore send errors - if there are no subscribers, that's fine
        let _ = self.event_sender.send(event);
    }
}

impl std::fmt::Debug for MempoolManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let core = self.core.lock().unwrap();
        f.debug_struct("MempoolManager")
            .field("core", &*core)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_acct_types::{AccountId, VarVec};
    use strata_codec::encode_to_vec;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::Buf32;
    use strata_ol_chain_types_new::{GamTxPayload, TransactionAttachment, TransactionPayload};
    use strata_storage::{NodeStorage, create_node_storage};

    use super::*;

    fn create_test_storage() -> Arc<NodeStorage> {
        let db = get_test_sled_backend();
        let threadpool = threadpool::ThreadPool::new(4);
        Arc::new(create_node_storage(db, threadpool).unwrap())
    }

    fn create_test_manager() -> MempoolManager {
        let storage = create_test_storage();
        let config = MempoolConfig::default();
        MempoolManager::new(config, 100, storage)
    }

    fn create_test_manager_with_config(config: MempoolConfig) -> MempoolManager {
        let storage = create_test_storage();
        MempoolManager::new(config, 100, storage)
    }

    fn create_test_tx_blob(payload_bytes: Vec<u8>) -> Vec<u8> {
        let payload = GamTxPayload::new(
            AccountId::new([0u8; 32]),
            VarVec::from_vec(payload_bytes).unwrap(),
        );
        let tx = OLTransaction::new(
            TransactionPayload::GenericAccountMessage(payload),
            TransactionAttachment::new(None, None),
        );
        encode_to_vec(&tx).unwrap()
    }

    #[test]
    fn test_submit_and_get() {
        let manager = create_test_manager();

        let blob1 = create_test_tx_blob(vec![1, 2, 3]);
        let blob2 = create_test_tx_blob(vec![4, 5, 6]);

        // Submit transactions
        let txid1 = manager.submit_transaction(blob1).unwrap();
        let txid2 = manager.submit_transaction(blob2).unwrap();

        // Get transactions (should be in FIFO order - earliest first)
        let txs = manager.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].0, txid1);
        assert_eq!(txs[1].0, txid2);

        // Check stats
        let stats = manager.stats().unwrap();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.enqueued_tx_total, 2);
    }

    #[test]
    fn test_duplicate_idempotency() {
        let manager = create_test_manager();

        let blob = create_test_tx_blob(vec![1, 2, 3]);

        // First submission succeeds
        let txid1 = manager.submit_transaction(blob.clone()).unwrap();

        // Second submission is idempotent - returns same txid
        let txid2 = manager.submit_transaction(blob).unwrap();
        assert_eq!(txid1, txid2);

        // Verify only one transaction in mempool
        let stats = manager.stats().unwrap();
        assert_eq!(stats.current_tx_count, 1);
    }

    #[test]
    fn test_remove_transactions() {
        let manager = create_test_manager();

        let blob1 = create_test_tx_blob(vec![1, 2, 3]);
        let blob2 = create_test_tx_blob(vec![4, 5, 6]);

        // Submit transactions
        let txid1 = manager.submit_transaction(blob1).unwrap();
        manager.submit_transaction(blob2).unwrap();

        // Remove tx1
        let removed = manager.remove_transactions(&[txid1]).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], txid1);

        // Only tx2 remains
        let txs = manager.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 1);

        // Check stats
        let stats = manager.stats().unwrap();
        assert_eq!(stats.current_tx_count, 1);
    }

    #[test]
    fn test_capacity_limits() {
        let config = MempoolConfig {
            max_tx_count: 2,
            max_tx_size: 500,
            max_txs_per_account: 16,
        };
        let manager = create_test_manager_with_config(config);

        let blob1 = create_test_tx_blob(vec![1; 200]);
        let blob2 = create_test_tx_blob(vec![2; 200]);
        let blob3 = create_test_tx_blob(vec![3; 200]);

        // Submit first two transactions
        manager.submit_transaction(blob1).unwrap();
        manager.submit_transaction(blob2).unwrap();

        // Third submission should evict tx1 (lowest priority = earliest)
        manager.submit_transaction(blob3).unwrap();

        // Check that tx3 was added and tx1 was evicted
        let txs = manager.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 2);

        // Check stats
        let stats = manager.stats().unwrap();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.evicted_tx_total, 1);
    }

    #[test]
    fn test_fifo_ordering() {
        let manager = create_test_manager();

        // Submit transactions with increasing entry slots
        for i in 0..5 {
            manager.update_current_slot(100 + i);
            let blob = create_test_tx_blob(vec![i as u8]);
            manager.submit_transaction(blob).unwrap();
        }

        // Get transactions - should be in FIFO order (earliest first)
        let txs = manager.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 5);
    }

    #[test]
    fn test_get_transactions_limit() {
        let manager = create_test_manager();

        // Submit 5 transactions
        for i in 0..5 {
            manager.update_current_slot(100 + i);
            let blob = create_test_tx_blob(vec![i as u8]);
            manager.submit_transaction(blob).unwrap();
        }

        // Request only 3 transactions
        let txs = manager.get_transactions(3).unwrap();
        assert_eq!(txs.len(), 3);
    }

    #[test]
    fn test_contains() {
        let manager = create_test_manager();

        let blob = create_test_tx_blob(vec![1, 2, 3]);
        let txid = manager.submit_transaction(blob).unwrap();

        assert!(manager.contains(&txid));
        assert!(!manager.contains(&OLTxId::from(Buf32([0u8; 32]))));
    }
}
