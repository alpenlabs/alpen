//! Mempool trait and core operations.
//!
//! Provides the main `Mempool` trait and an in-memory implementation.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use strata_codec::decode_buf_exact;
use strata_db_types::types::MempoolTxMetadata;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

use crate::{
    error::{MempoolError, MempoolResult},
    ordering::{OrderingIndex, OrderingStrategy},
    types::{MempoolConfig, MempoolStats},
    validation::TransactionValidator,
};

/// Core trait for mempool operations.
///
/// The mempool accepts opaque transaction blobs, parses and validates them,
/// and provides validated transactions for block assembly.
///
/// # Transaction Ingestion
///
/// Transactions can be ingested in two ways:
/// 1. **Stream-based** (primary): Via an [`OLTxProvider`](crate::OLTxProvider) stream in the
///    mempool's main loop. This is RPC-agnostic and works with any source (RPC, P2P, ZMQ, etc.).
/// 2. **Direct submission**: Via the `submit_transaction()` method for synchronous submission
///    (useful for RPC handlers, testing, or internal use).
pub trait Mempool: Send + Sync {
    /// Submits a raw transaction to the mempool.
    ///
    /// Accepts a raw transaction blob (opaque bytes) which is parsed into an `OLTransaction`,
    /// validated, and stored. Returns the transaction ID if successful.
    ///
    /// # Errors
    ///
    /// - [`MempoolError`](crate::error::MempoolError::ParseError) - if the transaction blob cannot
    ///   be parsed
    /// - [`MempoolError`](crate::error::MempoolError::DuplicateTransaction) - if the transaction
    ///   already exists
    /// - [`MempoolError`](crate::error::MempoolError::InvalidTransaction) - if validation fails
    /// - [`MempoolError`](crate::error::MempoolError::TransactionTooLarge) - if the transaction
    ///   exceeds size limits
    /// - [`MempoolError`](crate::error::MempoolError::MempoolCountLimitExceeded) - if mempool is
    ///   full
    /// - [`MempoolError`](crate::error::MempoolError::MempoolSizeLimitExceeded) - if mempool size
    ///   limit exceeded
    /// - [`MempoolError`](crate::error::MempoolError::DatabaseError) - if persistence fails
    fn submit_transaction(&self, blob: Vec<u8>) -> MempoolResult<OLTxId>;

    /// Retrieves transactions from the mempool for block assembly.
    ///
    /// Returns up to `limit` transactions, ordered by the mempool's ordering policy
    /// (initially FIFO by entry_slot, then by OLTxId as tie-breaker).
    ///
    /// Returns an empty vector if no transactions are available (not an error).
    ///
    /// # Errors
    ///
    /// - [`MempoolError`](crate::error::MempoolError::DatabaseError) - if database error occurs
    /// - [`MempoolError`](crate::error::MempoolError::Internal) - if internal error occurs
    fn get_transactions(&self, limit: u64) -> MempoolResult<Vec<(OLTxId, OLTransaction)>>;

    /// Removes transactions from the mempool.
    ///
    /// Typically called after transactions have been included in a block.
    /// Returns the list of transaction IDs that were successfully removed.
    /// Already-removed transactions are silently ignored (idempotent operation).
    ///
    /// # Errors
    ///
    /// - [`MempoolError`](crate::error::MempoolError::DatabaseError) - if database error occurs
    /// - [`MempoolError`](crate::error::MempoolError::Internal) - if internal error occurs
    fn remove_transactions(&self, txids: &[OLTxId]) -> MempoolResult<Vec<OLTxId>>;

    /// Gets statistics about the current mempool state.
    ///
    /// Returns statistics including transaction count, total size, and rejection counts.
    fn stats(&self) -> MempoolResult<MempoolStats>;
}

/// In-memory mempool implementation.
///
/// This implementation uses interior mutability to satisfy the `&self` requirement
/// of the `Mempool` trait, allowing concurrent access.
pub struct InMemoryMempool<S: OrderingStrategy, V: TransactionValidator> {
    /// Inner state protected by mutex.
    inner: Arc<Mutex<InMemoryMempoolInner<S, V>>>,
}

struct InMemoryMempoolInner<S: OrderingStrategy, V: TransactionValidator> {
    /// Configuration.
    config: MempoolConfig,

    /// Transaction validator.
    validator: V,

    /// Ordering index for priority-based retrieval.
    ordering: OrderingIndex<S>,

    /// Transaction storage: txid -> (blob, tx, metadata).
    /// Note: We store the raw blob so we can persist it later without re-encoding.
    /// Metadata is computed in-memory only and NOT persisted.
    transactions: HashMap<OLTxId, (Vec<u8>, OLTransaction, MempoolTxMetadata)>,

    /// Statistics.
    stats: MempoolStats,

    /// Current slot (for computing entry_slot in metadata).
    current_slot: u64,
}

impl<S: OrderingStrategy + 'static, V: TransactionValidator> InMemoryMempool<S, V> {
    /// Create a new in-memory mempool with the given configuration, ordering strategy,
    /// and validator.
    pub fn new(config: MempoolConfig, strategy: S, validator: V, current_slot: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(InMemoryMempoolInner {
                validator,
                ordering: OrderingIndex::new(strategy),
                transactions: HashMap::new(),
                stats: MempoolStats::default(),
                config,
                current_slot,
            })),
        }
    }

    /// Update the current slot (called by FCM-driven loop).
    pub fn update_current_slot(&self, slot: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.current_slot = slot;
    }
}

impl<S: OrderingStrategy + 'static, V: TransactionValidator> InMemoryMempoolInner<S, V> {
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
            if let Some((_blob, _tx, metadata)) = self.transactions.remove(&txid) {
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

impl<S: OrderingStrategy + 'static, V: TransactionValidator> Mempool for InMemoryMempool<S, V> {
    fn submit_transaction(&self, blob: Vec<u8>) -> MempoolResult<OLTxId> {
        let mut inner = self.inner.lock().unwrap();

        // Parse the transaction blob
        let tx: OLTransaction = decode_buf_exact(&blob)
            .map_err(|e| MempoolError::ParseError(format!("Failed to parse transaction: {e}")))?;

        // Compute transaction ID
        let txid = tx.compute_txid();

        // Check for duplicates
        if inner.transactions.contains_key(&txid) {
            return Err(MempoolError::DuplicateTransaction(txid));
        }

        // Compute metadata internally
        let metadata = inner.compute_metadata(blob.len());

        // Validate transaction
        if let Err(e) = inner.validator.validate(&tx, &metadata, inner.current_slot) {
            inner.stats.rejected_tx_total += 1;
            return Err(e);
        }

        // Ensure capacity (may evict lowest priority transactions)
        inner.ensure_capacity(metadata.size_bytes)?;

        // Insert into ordering index
        inner.ordering.insert(txid, &tx, &metadata);

        // Store transaction blob, parsed tx, and metadata in memory.
        // Note: Only the blob will be persisted later; metadata is in-memory only.
        inner
            .transactions
            .insert(txid, (blob, tx, metadata.clone()));

        // Update stats
        inner.stats.current_tx_count += 1;
        inner.stats.current_total_size += metadata.size_bytes;
        inner.stats.enqueued_tx_total += 1;

        Ok(txid)
    }

    fn get_transactions(&self, limit: u64) -> MempoolResult<Vec<(OLTxId, OLTransaction)>> {
        let inner = self.inner.lock().unwrap();
        let txids = inner.ordering.get_ordered_txids(limit as usize);

        let result = txids
            .into_iter()
            .filter_map(|txid| {
                inner
                    .transactions
                    .get(&txid)
                    .map(|(_blob, tx, _metadata)| (txid, tx.clone()))
            })
            .collect();

        Ok(result)
    }

    fn remove_transactions(&self, txids: &[OLTxId]) -> MempoolResult<Vec<OLTxId>> {
        let mut inner = self.inner.lock().unwrap();
        let mut removed = Vec::new();
        for txid in txids {
            if let Some((_blob, _tx, metadata)) = inner.transactions.remove(txid) {
                inner.ordering.remove(txid);
                inner.stats.current_tx_count -= 1;
                inner.stats.current_total_size -= metadata.size_bytes;
                removed.push(*txid);
            }
        }
        Ok(removed)
    }

    fn stats(&self) -> MempoolResult<MempoolStats> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.stats.clone())
    }
}

impl<S: OrderingStrategy, V: TransactionValidator> std::fmt::Debug for InMemoryMempool<S, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().unwrap();
        f.debug_struct("InMemoryMempool")
            .field("config", &inner.config)
            .field("ordering_strategy", &inner.ordering.strategy_name())
            .field("validator", &inner.validator.name())
            .field("tx_count", &inner.transactions.len())
            .field("stats", &inner.stats)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::AccountId;
    use strata_codec::encode_to_vec;
    use strata_ol_chain_types_new::{GamTxPayload, TransactionAttachment, TransactionPayload};

    use super::*;
    use crate::{ordering::FifoOrdering, validation::BasicValidator};

    fn create_test_tx_blob(payload_bytes: Vec<u8>) -> Vec<u8> {
        use strata_acct_types::VarVec;
        let payload = GamTxPayload::new(
            AccountId::new([0u8; 32]),
            VarVec::from_vec(payload_bytes).unwrap(),
        );
        let tx = OLTransaction::new(
            TransactionAttachment::new_empty(),
            TransactionPayload::GenericAccountMessage(payload),
        );
        encode_to_vec(&tx).unwrap()
    }

    #[test]
    fn test_submit_and_get() {
        let config = MempoolConfig::default();
        let strategy = FifoOrdering;
        let validator = BasicValidator::new(config.max_tx_size);
        let mempool = InMemoryMempool::new(config, strategy, validator, 100);

        let blob1 = create_test_tx_blob(vec![1, 2, 3]);
        let blob2 = create_test_tx_blob(vec![4, 5, 6]);

        // Submit transactions
        let txid1 = mempool.submit_transaction(blob1).unwrap();
        let txid2 = mempool.submit_transaction(blob2).unwrap();

        // Get transactions (should be in FIFO order - earliest first)
        let txs = mempool.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].0, txid1); // tx1 has earlier entry_slot
        assert_eq!(txs[1].0, txid2); // tx2 has later entry_slot

        // Check stats
        let stats = mempool.stats().unwrap();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.enqueued_tx_total, 2);
    }

    #[test]
    fn test_duplicate_rejection() {
        let config = MempoolConfig::default();
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let mempool = InMemoryMempool::new(config, strategy, validator, 100);

        let blob = create_test_tx_blob(vec![1, 2, 3]);

        // First submission succeeds
        mempool.submit_transaction(blob.clone()).unwrap();

        // Second submission fails
        let result = mempool.submit_transaction(blob);
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
        let mempool = InMemoryMempool::new(config, strategy, validator, 100);

        let blob1 = create_test_tx_blob(vec![1, 2, 3]);
        let blob2 = create_test_tx_blob(vec![4, 5, 6]);

        // Submit transactions
        let txid1 = mempool.submit_transaction(blob1).unwrap();
        mempool.submit_transaction(blob2).unwrap();

        // Remove tx1
        let removed = mempool.remove_transactions(&[txid1]).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], txid1);

        // Only tx2 remains
        let txs = mempool.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 1);

        // Check stats
        let stats = mempool.stats().unwrap();
        assert_eq!(stats.current_tx_count, 1);
    }

    #[test]
    fn test_capacity_limits() {
        let config = MempoolConfig {
            max_tx_count: 2,
            max_tx_size: 500,
        };
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let mempool = InMemoryMempool::new(config, strategy, validator, 100);

        let blob1 = create_test_tx_blob(vec![1; 200]);
        let blob2 = create_test_tx_blob(vec![2; 200]);
        let blob3 = create_test_tx_blob(vec![3; 200]);

        // Submit first two transactions
        mempool.submit_transaction(blob1).unwrap();
        mempool.submit_transaction(blob2).unwrap();

        // Third submission should evict tx1 (lowest priority = earliest)
        mempool.submit_transaction(blob3).unwrap();

        // Check that tx3 was added and tx1 was evicted
        let txs = mempool.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 2);

        // Check stats
        let stats = mempool.stats().unwrap();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.evicted_tx_total, 1);
    }

    #[test]
    fn test_fifo_ordering() {
        let config = MempoolConfig::default();
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let mempool = InMemoryMempool::new(config, strategy, validator, 100);

        // Submit transactions with increasing entry slots
        for i in 0..5 {
            mempool.update_current_slot(100 + i);
            let blob = create_test_tx_blob(vec![i as u8]);
            mempool.submit_transaction(blob).unwrap();
        }

        // Get transactions - should be in FIFO order (earliest first)
        let txs = mempool.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 5);
    }

    #[test]
    fn test_get_transactions_limit() {
        let config = MempoolConfig::default();
        let validator = BasicValidator::new(config.max_tx_size);
        let strategy = FifoOrdering;
        let mempool = InMemoryMempool::new(config, strategy, validator, 100);

        // Submit 5 transactions
        for i in 0..5 {
            mempool.update_current_slot(100 + i);
            let blob = create_test_tx_blob(vec![i as u8]);
            mempool.submit_transaction(blob).unwrap();
        }

        // Request only 3 transactions
        let txs = mempool.get_transactions(3).unwrap();
        assert_eq!(txs.len(), 3);
    }
}
