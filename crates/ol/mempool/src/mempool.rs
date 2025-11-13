//! Core mempool implementation.
//!
//! Provides the main `Mempool` trait and an in-memory implementation with database persistence.

use std::collections::HashMap;

use strata_db_types::traits::MempoolDatabase;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

use crate::{
    MempoolError, MempoolResult, MempoolTxMetadata,
    ordering::{OrderingIndex, OrderingStrategy},
    types::{MempoolConfig, MempoolStats},
    validation::TransactionValidator,
};

/// Mempool interface for managing pending transactions.
pub trait Mempool: Send + Sync {
    /// Submit a transaction to the mempool.
    ///
    /// Returns the transaction ID on success.
    ///
    /// # Errors
    ///
    /// - [`MempoolError::DuplicateTransaction`] if transaction already exists
    /// - [`MempoolError::TransactionTooLarge`], [`MempoolError::TooEarly`],
    ///   [`MempoolError::Expired`], or [`MempoolError::InvalidTransaction`] if validation fails
    /// - [`MempoolError::MempoolCountLimitExceeded`] or [`MempoolError::MempoolSizeLimitExceeded`]
    ///   if mempool is full (after eviction attempts)
    fn submit_transaction(
        &mut self,
        tx: OLTransaction,
        metadata: MempoolTxMetadata,
        current_slot: u64,
    ) -> MempoolResult<OLTxId>;

    /// Get up to `limit` transactions in priority order.
    ///
    /// Returns transactions and their metadata ordered by priority (highest first).
    ///
    /// # Errors
    ///
    /// - [`MempoolError::DatabaseError`] if database operations fail
    fn get_transactions(
        &self,
        limit: usize,
    ) -> MempoolResult<Vec<(OLTxId, OLTransaction, MempoolTxMetadata)>>;

    /// Remove transactions from the mempool.
    ///
    /// Typically called after transactions are included in a block.
    /// Returns the list of transaction IDs that were actually removed.
    ///
    /// # Errors
    ///
    /// - [`MempoolError::DatabaseError`] if database deletion fails
    fn remove_transactions(&mut self, txids: &[OLTxId]) -> MempoolResult<Vec<OLTxId>>;

    /// Get mempool statistics.
    ///
    /// # Errors
    ///
    /// - [`MempoolError::DatabaseError`] if database operations fail
    fn stats(&self) -> MempoolResult<MempoolStats>;
}

/// In-memory mempool implementation with database persistence.
pub struct InMemoryMempool<S: OrderingStrategy, V: TransactionValidator, D: MempoolDatabase> {
    /// Configuration.
    config: MempoolConfig,

    /// Transaction validator.
    validator: V,

    /// Ordering index for priority-based retrieval.
    ordering: OrderingIndex<S>,

    /// Transaction storage: txid -> (tx, metadata).
    transactions: HashMap<OLTxId, (OLTransaction, MempoolTxMetadata)>,

    /// Database for persistence.
    database: D,

    /// Statistics.
    stats: MempoolStats,
}

impl<S: OrderingStrategy, V: TransactionValidator, D: MempoolDatabase> std::fmt::Debug
    for InMemoryMempool<S, V, D>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryMempool")
            .field("config", &self.config)
            .field("ordering_strategy", &self.ordering.strategy_name())
            .field("validator", &self.validator.name())
            .field("tx_count", &self.transactions.len())
            .field("stats", &self.stats)
            .finish()
    }
}

impl<S: OrderingStrategy + 'static, V: TransactionValidator, D: MempoolDatabase>
    InMemoryMempool<S, V, D>
{
    /// Create a new in-memory mempool with the given configuration, ordering strategy,
    /// validator, and database.
    pub fn new(config: MempoolConfig, strategy: S, validator: V, database: D) -> Self {
        Self {
            validator,
            ordering: OrderingIndex::new(strategy),
            transactions: HashMap::new(),
            database,
            stats: MempoolStats::default(),
            config,
        }
    }

    /// Create a new mempool and restore state from database.
    ///
    /// # Errors
    ///
    /// Returns [`MempoolError::DatabaseError`] if database operations fail.
    pub fn new_with_restore(
        config: MempoolConfig,
        strategy: S,
        validator: V,
        database: D,
        current_slot: u64,
    ) -> MempoolResult<Self> {
        let mut mempool = Self::new(config, strategy, validator, database);
        mempool.restore_from_database(current_slot)?;
        Ok(mempool)
    }

    /// Restore mempool state from database.
    fn restore_from_database(&mut self, current_slot: u64) -> MempoolResult<()> {
        let txids = self
            .database
            .get_all_tx_ids()
            .map_err(|e| MempoolError::DatabaseError(e.to_string()))?;

        for txid in txids {
            if let Some((tx, metadata)) = self
                .database
                .get_tx_entry(&txid)
                .map_err(|e| MempoolError::DatabaseError(e.to_string()))?
            {
                // Re-validate transaction (may have expired)
                if self
                    .validator
                    .validate(&tx, &metadata, current_slot)
                    .is_ok()
                {
                    // Add to ordering index
                    self.ordering.insert(txid, &tx, &metadata);

                    // Add to in-memory storage
                    self.transactions.insert(txid, (tx, metadata.clone()));

                    // Update stats
                    self.stats.current_tx_count += 1;
                    self.stats.current_total_size += metadata.size_bytes;
                } else {
                    // Transaction no longer valid - remove from database
                    let _ = self.database.del_tx_entry(&txid);
                }
            }
        }

        Ok(())
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
    ///
    /// # Errors
    ///
    /// Returns [`MempoolError::DatabaseError`] if database deletion fails.
    fn try_evict_one(&mut self) -> MempoolResult<bool> {
        // Get the lowest priority transaction (last in ordering)
        let lowest_priority_txids = self.ordering.get_ordered_txids(usize::MAX);

        if let Some(txid) = lowest_priority_txids.last() {
            let txid = *txid;
            if let Some((_, metadata)) = self.transactions.remove(&txid) {
                // Remove from database
                self.database
                    .del_tx_entry(&txid)
                    .map_err(|e| MempoolError::DatabaseError(e.to_string()))?;

                self.ordering.remove(&txid);
                self.stats.current_tx_count -= 1;
                self.stats.current_total_size -= metadata.size_bytes;
                self.stats.evicted_tx_total += 1;
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Try to make room for a transaction of the given size.
    ///
    /// Evicts lowest priority transactions until enough space is available.
    fn ensure_capacity(&mut self, tx_size: usize) -> MempoolResult<()> {
        // Try evicting transactions one by one until we have capacity
        while !self.has_capacity(tx_size) {
            if !self.try_evict_one()? {
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
}

impl<S: OrderingStrategy + 'static, V: TransactionValidator, D: MempoolDatabase> Mempool
    for InMemoryMempool<S, V, D>
{
    fn submit_transaction(
        &mut self,
        tx: OLTransaction,
        metadata: MempoolTxMetadata,
        current_slot: u64,
    ) -> MempoolResult<OLTxId> {
        // Compute transaction ID
        let txid = tx.compute_txid();

        // Check for duplicates
        if self.transactions.contains_key(&txid) {
            return Err(MempoolError::DuplicateTransaction(txid));
        }

        // Validate transaction
        if let Err(e) = self.validator.validate(&tx, &metadata, current_slot) {
            self.stats.rejected_tx_total += 1;
            return Err(e);
        }

        // Ensure capacity (may evict lowest priority transactions)
        self.ensure_capacity(metadata.size_bytes)?;

        // Persist to database first
        self.database
            .put_tx_entry(&txid, &tx, &metadata)
            .map_err(|e| MempoolError::DatabaseError(e.to_string()))?;

        // Insert into ordering index
        self.ordering.insert(txid, &tx, &metadata);

        // Store transaction in memory
        self.transactions.insert(txid, (tx, metadata.clone()));

        // Update stats
        self.stats.current_tx_count += 1;
        self.stats.current_total_size += metadata.size_bytes;
        self.stats.enqueued_tx_total += 1;

        Ok(txid)
    }

    fn get_transactions(
        &self,
        limit: usize,
    ) -> MempoolResult<Vec<(OLTxId, OLTransaction, MempoolTxMetadata)>> {
        let txids = self.ordering.get_ordered_txids(limit);

        let result = txids
            .into_iter()
            .filter_map(|txid| {
                self.transactions
                    .get(&txid)
                    .map(|(tx, metadata)| (txid, tx.clone(), metadata.clone()))
            })
            .collect();

        Ok(result)
    }

    fn remove_transactions(&mut self, txids: &[OLTxId]) -> MempoolResult<Vec<OLTxId>> {
        // Remove from database first (batch operation)
        self.database
            .del_tx_entries(txids)
            .map_err(|e| MempoolError::DatabaseError(e.to_string()))?;

        let mut removed = Vec::new();
        for txid in txids {
            if let Some((_, metadata)) = self.transactions.remove(txid) {
                self.ordering.remove(txid);
                self.stats.current_tx_count -= 1;
                self.stats.current_total_size -= metadata.size_bytes;
                removed.push(*txid);
            }
        }
        Ok(removed)
    }

    fn stats(&self) -> MempoolResult<MempoolStats> {
        Ok(self.stats.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use strata_acct_types::AccountId;
    use strata_db_types::DbResult;
    use strata_ol_chain_types_new::{TransactionExtra, TransactionPayload};

    use super::*;
    use crate::{ordering::FifoOrdering, validation::BasicValidator};

    /// Mock database implementation for testing (no-op).
    #[derive(Debug, Clone)]
    struct MockDatabase;

    impl MempoolDatabase for MockDatabase {
        fn put_tx_entry(
            &self,
            _txid: &OLTxId,
            _tx: &OLTransaction,
            _metadata: &MempoolTxMetadata,
        ) -> DbResult<()> {
            Ok(())
        }

        fn get_tx_entry(
            &self,
            _txid: &OLTxId,
        ) -> DbResult<Option<(OLTransaction, MempoolTxMetadata)>> {
            Ok(None)
        }

        fn get_tx_entries(
            &self,
            _txids: &[OLTxId],
        ) -> DbResult<HashMap<OLTxId, (OLTransaction, MempoolTxMetadata)>> {
            Ok(HashMap::new())
        }

        fn del_tx_entry(&self, _txid: &OLTxId) -> DbResult<()> {
            Ok(())
        }

        fn del_tx_entries(&self, _txids: &[OLTxId]) -> DbResult<()> {
            Ok(())
        }

        fn get_all_tx_ids(&self) -> DbResult<Vec<OLTxId>> {
            Ok(Vec::new())
        }
    }

    fn create_test_tx(payload_bytes: Vec<u8>) -> OLTransaction {
        OLTransaction::new(
            TransactionPayload::GenericAccountMessage {
                target: AccountId::new([0u8; 32]),
                payload: payload_bytes,
            },
            TransactionExtra::new(None, None),
        )
    }

    fn create_test_metadata(entry_slot: u64, size_bytes: usize) -> MempoolTxMetadata {
        MempoolTxMetadata {
            entry_slot,
            entry_time: 0,
            size_bytes,
        }
    }

    #[test]
    fn test_submit_and_get() {
        let config = MempoolConfig::default();
        let strategy = FifoOrdering;
        let validator = BasicValidator::new(config.max_tx_size);
        let database = MockDatabase;
        let mut mempool = InMemoryMempool::new(config, strategy, validator, database);

        let tx1 = create_test_tx(vec![1, 2, 3]);
        let metadata1 = create_test_metadata(100, 100);

        let tx2 = create_test_tx(vec![4, 5, 6]);
        let metadata2 = create_test_metadata(101, 150);

        // Submit transactions
        mempool
            .submit_transaction(tx1.clone(), metadata1, 100)
            .unwrap();
        mempool
            .submit_transaction(tx2.clone(), metadata2, 101)
            .unwrap();

        // Get transactions (should be in FIFO order - earliest first)
        let txs = mempool.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].2.entry_slot, 100); // tx1 has earlier entry_slot
        assert_eq!(txs[1].2.entry_slot, 101); // tx2 has later entry_slot

        // Check stats
        let stats = mempool.stats().unwrap();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.current_total_size, 250);
        assert_eq!(stats.enqueued_tx_total, 2);
    }

    #[test]
    fn test_duplicate_rejection() {
        let config = MempoolConfig::default();
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let database = MockDatabase;
        let mut mempool = InMemoryMempool::new(config, strategy, validator, database);

        let tx = create_test_tx(vec![1, 2, 3]);
        let metadata = create_test_metadata(100, 100);

        // First submission succeeds
        mempool
            .submit_transaction(tx.clone(), metadata.clone(), 100)
            .unwrap();

        // Second submission fails
        let result = mempool.submit_transaction(tx, metadata, 100);
        assert!(matches!(
            result,
            Err(MempoolError::DuplicateTransaction { .. })
        ));
    }

    #[test]
    fn test_remove_transactions() {
        let config = MempoolConfig::default();
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let database = MockDatabase;
        let mut mempool = InMemoryMempool::new(config, strategy, validator, database);

        let tx1 = create_test_tx(vec![1, 2, 3]);
        let metadata1 = create_test_metadata(100, 100);
        let txid1 = tx1.compute_txid();

        let tx2 = create_test_tx(vec![4, 5, 6]);
        let metadata2 = create_test_metadata(101, 150);

        // Submit transactions
        mempool.submit_transaction(tx1, metadata1, 100).unwrap();
        mempool
            .submit_transaction(tx2.clone(), metadata2, 101)
            .unwrap();

        // Remove tx1
        let removed = mempool.remove_transactions(&[txid1]).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], txid1);

        // Only tx2 remains
        let txs = mempool.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].2.entry_slot, 101);

        // Check stats
        let stats = mempool.stats().unwrap();
        assert_eq!(stats.current_tx_count, 1);
        assert_eq!(stats.current_total_size, 150);
    }

    #[test]
    fn test_capacity_limits() {
        let config = MempoolConfig {
            max_tx_count: 2,
            max_tx_size: 500,
        };
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let database = MockDatabase;
        let mut mempool = InMemoryMempool::new(config, strategy, validator, database);

        let tx1 = create_test_tx(vec![1; 200]);
        let metadata1 = create_test_metadata(100, 200);

        let tx2 = create_test_tx(vec![2; 200]);
        let metadata2 = create_test_metadata(101, 200);

        let tx3 = create_test_tx(vec![3; 200]);
        let metadata3 = create_test_metadata(102, 200);

        // Submit first two transactions
        mempool.submit_transaction(tx1, metadata1, 100).unwrap();
        mempool.submit_transaction(tx2, metadata2, 101).unwrap();

        // Third submission should evict tx1 (lowest priority = earliest)
        mempool
            .submit_transaction(tx3.clone(), metadata3, 102)
            .unwrap();

        // Check that tx3 was added and tx1 was evicted
        let txs = mempool.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 2);

        // Check stats
        let stats = mempool.stats().unwrap();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.evicted_tx_total, 1);
    }

    #[test]
    fn test_transaction_too_large() {
        let config = MempoolConfig {
            max_tx_count: 10,
            max_tx_size: 100,
        };
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let database = MockDatabase;
        let mut mempool = InMemoryMempool::new(config, strategy, validator, database);

        let tx = create_test_tx(vec![1; 50]);
        let metadata = create_test_metadata(100, 200); // size_bytes exceeds max

        let result = mempool.submit_transaction(tx, metadata, 100);
        assert!(matches!(
            result,
            Err(MempoolError::TransactionTooLarge { .. })
        ));

        let stats = mempool.stats().unwrap();
        assert_eq!(stats.rejected_tx_total, 1);
    }

    #[test]
    fn test_fifo_ordering() {
        let config = MempoolConfig::default();
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let database = MockDatabase;
        let mut mempool = InMemoryMempool::new(config, strategy, validator, database);

        // Submit transactions with increasing entry slots
        for i in 0..5 {
            let tx = create_test_tx(vec![i as u8]);
            let metadata = create_test_metadata(100 + i, 100);
            mempool.submit_transaction(tx, metadata, 100 + i).unwrap();
        }

        // Get transactions - should be in FIFO order (earliest first)
        let txs = mempool.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 5);

        for (i, (_txid, _tx, metadata)) in txs.iter().enumerate() {
            assert_eq!(metadata.entry_slot, 100 + i as u64);
        }
    }

    #[test]
    fn test_get_transactions_limit() {
        let config = MempoolConfig::default();
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let database = MockDatabase;
        let mut mempool = InMemoryMempool::new(config, strategy, validator, database);

        // Submit 5 transactions
        for i in 0..5 {
            let tx = create_test_tx(vec![i as u8]);
            let metadata = create_test_metadata(100 + i, 100);
            mempool.submit_transaction(tx, metadata, 100 + i).unwrap();
        }

        // Request only 3 transactions
        let txs = mempool.get_transactions(3).unwrap();
        assert_eq!(txs.len(), 3);

        // Should get the 3 earliest (highest priority in FIFO)
        for (i, (_txid, _tx, metadata)) in txs.iter().enumerate() {
            assert_eq!(metadata.entry_slot, 100 + i as u64);
        }
    }
}
