//! Core mempool data structure.
//!
//! Provides a pure data structure for managing pending transactions without
//! external dependencies (no Arc/Mutex, no database).

use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use strata_db_types::types::MempoolTxMetadata;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

use crate::{
    error::{MempoolError, MempoolResult},
    ordering::{FifoOrdering, OrderingIndex, OrderingStrategy},
    types::{MempoolConfig, MempoolStats},
    validation::{BasicValidator, TransactionValidator},
};

/// Core mempool data structure.
///
/// This is a pure data structure with no external dependencies. It uses hardcoded
/// strategies (FifoOrdering, BasicValidator) and exposes methods that operate on
/// `&mut self`.
///
/// The `MempoolManager` wraps this with Arc<Mutex<>> and provides a synchronous API.
pub struct MempoolCore {
    /// Configuration.
    config: MempoolConfig,

    /// Transaction validator (hardcoded BasicValidator).
    validator: BasicValidator,

    /// Ordering index for priority-based retrieval (hardcoded FifoOrdering).
    ordering: OrderingIndex<FifoOrdering>,

    /// Transaction storage: txid -> (tx, metadata).
    transactions: HashMap<OLTxId, (OLTransaction, MempoolTxMetadata)>,

    /// Statistics.
    stats: MempoolStats,

    /// Current slot (for computing entry_slot in metadata).
    current_slot: u64,
}

impl MempoolCore {
    /// Create a new mempool core with the given configuration and current slot.
    pub fn new(config: MempoolConfig, current_slot: u64) -> Self {
        let validator = BasicValidator::new(config.max_tx_size);
        let ordering = OrderingIndex::new(FifoOrdering);

        Self {
            config,
            validator,
            ordering,
            transactions: HashMap::new(),
            stats: MempoolStats::default(),
            current_slot,
        }
    }

    /// Update the current slot (called by FCM-driven loop).
    pub fn update_current_slot(&mut self, slot: u64) {
        self.current_slot = slot;
    }

    /// Get current slot.
    pub fn current_slot(&self) -> u64 {
        self.current_slot
    }

    /// Submit a transaction to the mempool.
    ///
    /// Returns the transaction ID and computed priority if successful.
    /// Idempotent: returns success if transaction already exists (with priority 0 for existing).
    pub fn submit_transaction(
        &mut self,
        tx: OLTransaction,
        blob_size: usize,
    ) -> MempoolResult<(OLTxId, u64)> {
        // Compute transaction ID
        let txid = tx.compute_txid();

        // Check for duplicates - idempotent operation, return success if already exists
        // Priority is 0 for existing transactions (not re-computed)
        if self.transactions.contains_key(&txid) {
            return Ok((txid, 0));
        }

        // Compute metadata
        let metadata = self.compute_metadata(blob_size);

        // Validate transaction
        if let Err(e) = self.validator.validate(&tx, &metadata, self.current_slot) {
            self.stats.rejected_tx_total += 1;
            return Err(e);
        }

        // Ensure capacity (may evict lowest priority transactions)
        self.ensure_capacity(metadata.size_bytes)?;

        // Compute priority (before inserting, following original design)
        let priority = self.ordering.strategy().compute_priority(&tx, &metadata);

        // Insert into ordering index
        self.ordering.insert(txid, &tx, &metadata);

        // Store transaction and metadata
        self.transactions.insert(txid, (tx, metadata.clone()));

        // Update stats
        self.stats.current_tx_count += 1;
        self.stats.current_total_size += metadata.size_bytes;
        self.stats.enqueued_tx_total += 1;

        Ok((txid, priority))
    }

    /// Retrieve transactions for block assembly.
    ///
    /// Returns up to `limit` transactions in priority order.
    pub fn get_transactions(&self, limit: u64) -> Vec<(OLTxId, OLTransaction)> {
        let txids = self.ordering.get_ordered_txids(limit as usize);

        txids
            .into_iter()
            .filter_map(|txid| {
                self.transactions
                    .get(&txid)
                    .map(|(tx, _metadata)| (txid, tx.clone()))
            })
            .collect()
    }

    /// Remove transactions from the mempool.
    ///
    /// Returns the list of transaction IDs that were successfully removed.
    pub fn remove_transactions(&mut self, txids: &[OLTxId]) -> Vec<OLTxId> {
        let mut removed = Vec::new();
        for txid in txids {
            if let Some((_tx, metadata)) = self.transactions.remove(txid) {
                self.ordering.remove(txid);
                self.stats.current_tx_count -= 1;
                self.stats.current_total_size -= metadata.size_bytes;
                removed.push(*txid);
            }
        }
        removed
    }

    /// Get current mempool statistics.
    pub fn stats(&self) -> MempoolStats {
        self.stats.clone()
    }

    /// Check if a transaction exists in the mempool.
    pub fn contains(&self, txid: &OLTxId) -> bool {
        self.transactions.contains_key(txid)
    }

    /// Iterate over all transactions in the mempool.
    ///
    /// Returns an iterator over (txid, transaction, metadata) tuples.
    pub fn transactions(
        &self,
    ) -> impl Iterator<Item = (&OLTxId, &OLTransaction, &MempoolTxMetadata)> {
        self.transactions
            .iter()
            .map(|(txid, (tx, metadata))| (txid, tx, metadata))
    }

    /// Check if mempool has capacity for a transaction of the given size.
    fn has_capacity(&self, tx_size: usize) -> bool {
        let new_count = self.stats.current_tx_count + 1;
        let new_total_size = self.stats.current_total_size + tx_size;

        new_count <= self.config.max_tx_count && new_total_size <= self.config.max_total_size()
    }

    /// Try to evict lowest priority transaction to make room.
    ///
    /// Returns true if eviction succeeded, false if no transactions to evict.
    fn try_evict_one(&mut self) -> bool {
        // Get the lowest priority transaction (last in ordering)
        let lowest_priority_txids = self.ordering.get_ordered_txids(usize::MAX);

        if let Some(txid) = lowest_priority_txids.last() {
            let txid = *txid;
            if let Some((_tx, metadata)) = self.transactions.remove(&txid) {
                self.ordering.remove(&txid);
                self.stats.current_tx_count -= 1;
                self.stats.current_total_size -= metadata.size_bytes;
                self.stats.evicted_tx_total += 1;
                return true;
            }
        }

        false
    }

    /// Try to make room for a transaction of the given size.
    ///
    /// Evicts lowest priority transactions until enough space is available.
    fn ensure_capacity(&mut self, tx_size: usize) -> MempoolResult<()> {
        // Try evicting transactions one by one until we have capacity
        while !self.has_capacity(tx_size) {
            if !self.try_evict_one() {
                // No more transactions to evict - determine which limit we hit
                if self.stats.current_tx_count >= self.config.max_tx_count {
                    return Err(MempoolError::MempoolCountLimitExceeded {
                        count: self.stats.current_tx_count,
                        max: self.config.max_tx_count,
                    });
                } else {
                    return Err(MempoolError::MempoolSizeLimitExceeded {
                        size: self.stats.current_total_size,
                        max: self.config.max_total_size(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Compute metadata for a transaction blob.
    fn compute_metadata(&self, blob_size: usize) -> MempoolTxMetadata {
        let entry_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        MempoolTxMetadata {
            entry_slot: self.current_slot,
            entry_time,
            size_bytes: blob_size,
        }
    }
}

impl std::fmt::Debug for MempoolCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MempoolCore")
            .field("config", &self.config)
            .field("ordering_strategy", &self.ordering.strategy_name())
            .field("validator", &self.validator.name())
            .field("tx_count", &self.transactions.len())
            .field("stats", &self.stats)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, VarVec};
    use strata_codec::encode_to_vec;
    use strata_identifiers::Buf32;
    use strata_ol_chain_types_new::{GamTxPayload, TransactionAttachment, TransactionPayload};

    use super::*;

    fn create_test_tx(payload_bytes: Vec<u8>) -> (OLTransaction, usize) {
        let payload = GamTxPayload::new(
            AccountId::new([0u8; 32]),
            VarVec::from_vec(payload_bytes).unwrap(),
        );
        let tx = OLTransaction::new(
            TransactionPayload::GenericAccountMessage(payload),
            TransactionAttachment::new(None, None),
        );
        let blob = encode_to_vec(&tx).unwrap();
        let blob_size = blob.len();
        (tx, blob_size)
    }

    #[test]
    fn test_submit_and_get() {
        let config = MempoolConfig::default();
        let mut core = MempoolCore::new(config, 100);

        let (tx1, size1) = create_test_tx(vec![1, 2, 3]);
        let (tx2, size2) = create_test_tx(vec![4, 5, 6]);

        // Submit transactions
        let (txid1, priority1) = core.submit_transaction(tx1, size1).unwrap();
        let (txid2, priority2) = core.submit_transaction(tx2, size2).unwrap();

        // Both submitted at same slot, should have equal priority
        assert_eq!(priority1, priority2);

        // Get transactions (should be in FIFO order - earliest first)
        let txs = core.get_transactions(10);
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].0, txid1);
        assert_eq!(txs[1].0, txid2);

        // Check stats
        let stats = core.stats();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.enqueued_tx_total, 2);
    }

    #[test]
    fn test_duplicate_idempotency() {
        let config = MempoolConfig::default();
        let mut core = MempoolCore::new(config, 100);

        let (tx, size) = create_test_tx(vec![1, 2, 3]);

        // First submission succeeds
        let (txid1, _priority1) = core.submit_transaction(tx.clone(), size).unwrap();

        // Second submission is idempotent - returns same txid
        let (txid2, _priority2) = core.submit_transaction(tx, size).unwrap();
        assert_eq!(txid1, txid2);

        // Verify only one transaction in mempool
        let stats = core.stats();
        assert_eq!(stats.current_tx_count, 1);
    }

    #[test]
    fn test_remove_transactions() {
        let config = MempoolConfig::default();
        let mut core = MempoolCore::new(config, 100);

        let (tx1, size1) = create_test_tx(vec![1, 2, 3]);
        let (tx2, size2) = create_test_tx(vec![4, 5, 6]);

        // Submit transactions
        let (txid1, _priority1) = core.submit_transaction(tx1, size1).unwrap();
        core.submit_transaction(tx2, size2).unwrap();

        // Remove tx1
        let removed = core.remove_transactions(&[txid1]);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], txid1);

        // Only tx2 remains
        let txs = core.get_transactions(10);
        assert_eq!(txs.len(), 1);

        // Check stats
        let stats = core.stats();
        assert_eq!(stats.current_tx_count, 1);
    }

    #[test]
    fn test_capacity_limits() {
        let config = MempoolConfig {
            max_tx_count: 2,
            max_tx_size: 500,
            max_txs_per_account: 16,
        };
        let mut core = MempoolCore::new(config, 100);

        let (tx1, size1) = create_test_tx(vec![1; 200]);
        let (tx2, size2) = create_test_tx(vec![2; 200]);
        let (tx3, size3) = create_test_tx(vec![3; 200]);

        // Submit first two transactions
        let (_txid1, _priority1) = core.submit_transaction(tx1, size1).unwrap();
        let (_txid2, _priority2) = core.submit_transaction(tx2, size2).unwrap();

        // Third submission should evict tx1 (lowest priority = earliest)
        let (_txid3, _priority3) = core.submit_transaction(tx3, size3).unwrap();

        // Check that tx3 was added and tx1 was evicted
        let txs = core.get_transactions(10);
        assert_eq!(txs.len(), 2);

        // Check stats
        let stats = core.stats();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.evicted_tx_total, 1);
    }

    #[test]
    fn test_fifo_ordering() {
        let config = MempoolConfig::default();
        let mut core = MempoolCore::new(config, 100);

        // Submit transactions with increasing entry slots
        let mut priorities = Vec::new();
        for i in 0..5 {
            core.update_current_slot(100 + i);
            let (tx, size) = create_test_tx(vec![i as u8]);
            let (_txid, priority) = core.submit_transaction(tx, size).unwrap();
            priorities.push(priority);
        }

        // Verify priorities decrease as slot increases (FIFO = earlier is higher priority)
        for i in 1..5 {
            assert!(priorities[i - 1] > priorities[i]);
        }

        // Get transactions - should be in FIFO order (earliest first)
        let txs = core.get_transactions(10);
        assert_eq!(txs.len(), 5);
    }

    #[test]
    fn test_get_transactions_limit() {
        let config = MempoolConfig::default();
        let mut core = MempoolCore::new(config, 100);

        // Submit 5 transactions
        for i in 0..5 {
            core.update_current_slot(100 + i);
            let (tx, size) = create_test_tx(vec![i as u8]);
            core.submit_transaction(tx, size).unwrap();
        }

        // Request only 3 transactions
        let txs = core.get_transactions(3);
        assert_eq!(txs.len(), 3);
    }

    #[test]
    fn test_contains() {
        let config = MempoolConfig::default();
        let mut core = MempoolCore::new(config, 100);

        let (tx, size) = create_test_tx(vec![1, 2, 3]);
        let (txid, _priority) = core.submit_transaction(tx, size).unwrap();

        assert!(core.contains(&txid));
        assert!(!core.contains(&OLTxId::from(Buf32([0u8; 32]))));
    }
}
