//! Transaction ordering strategies for mempool.
//!
//! Provides pluggable ordering strategies to determine transaction priority.

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Debug,
};

use strata_db_types::types::MempoolTxMetadata;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

/// Strategy for ordering transactions in the mempool.
///
/// Different strategies can prioritize transactions differently:
/// - FIFO: First in, first out (simple, predictable)
/// - Fee-based: Higher fees get priority (incentive-compatible)
/// - Hybrid: Combination of fee and time-based ordering
pub trait OrderingStrategy: Send + Sync {
    /// Compute priority for a transaction.
    ///
    /// Higher priority values mean the transaction should be included first.
    /// Returns a u64 priority score.
    fn compute_priority(&self, tx: &OLTransaction, metadata: &MempoolTxMetadata) -> u64;

    /// Name of the strategy (for logging/metrics).
    fn name(&self) -> &'static str;
}

/// FIFO (First In, First Out) ordering strategy.
///
/// Transactions are ordered by entry slot - earlier transactions get priority.
/// This is the simplest and most predictable strategy.
#[derive(Debug, Clone, Copy)]
pub struct FifoOrdering;

impl OrderingStrategy for FifoOrdering {
    fn compute_priority(&self, _tx: &OLTransaction, metadata: &MempoolTxMetadata) -> u64 {
        // Invert entry_slot so earlier slots have higher priority
        // u64::MAX - slot ensures earlier transactions sort higher
        u64::MAX - metadata.entry_slot
    }

    fn name(&self) -> &'static str {
        "fifo"
    }
}

/// Efficient index for ordering transactions by priority.
///
/// Uses a BTreeMap to maintain transactions sorted by (priority, txid).
/// The txid is included in the key to handle ties and enable efficient removal.
pub struct OrderingIndex<S> {
    /// The ordering strategy used to compute priorities.
    strategy: S,

    /// Priority queue: (priority, txid) -> ()
    /// BTreeMap keeps entries sorted by key, enabling efficient iteration.
    priority_queue: BTreeMap<(u64, OLTxId), ()>,

    /// Reverse index: txid -> priority
    /// Enables O(1) lookup of transaction priority for updates/removals.
    reverse_index: HashMap<OLTxId, u64>,
}

impl<S: OrderingStrategy> Debug for OrderingIndex<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrderingIndex")
            .field("strategy", &self.strategy.name())
            .field("tx_count", &self.priority_queue.len())
            .finish()
    }
}

impl<S: OrderingStrategy> OrderingIndex<S> {
    /// Create a new ordering index with the given strategy.
    pub fn new(strategy: S) -> Self {
        Self {
            strategy,
            priority_queue: BTreeMap::new(),
            reverse_index: HashMap::new(),
        }
    }

    /// Insert a transaction into the ordering index.
    ///
    /// Computes priority using the strategy and adds to both indices.
    pub fn insert(&mut self, txid: OLTxId, tx: &OLTransaction, metadata: &MempoolTxMetadata) {
        let priority = self.strategy.compute_priority(tx, metadata);

        self.priority_queue.insert((priority, txid), ());
        self.reverse_index.insert(txid, priority);
    }

    /// Remove a transaction from the ordering index.
    pub fn remove(&mut self, txid: &OLTxId) {
        if let Some(priority) = self.reverse_index.remove(txid) {
            self.priority_queue.remove(&(priority, *txid));
        }
    }

    /// Get transaction IDs in priority order (highest priority first).
    ///
    /// Returns up to `limit` transaction IDs, ordered by descending priority.
    pub fn get_ordered_txids(&self, limit: usize) -> Vec<OLTxId> {
        self.priority_queue
            .iter()
            .rev() // Reverse to get highest priority first
            .take(limit)
            .map(|((_, txid), _)| *txid)
            .collect()
    }

    /// Get the number of transactions in the index.
    pub fn len(&self) -> usize {
        self.priority_queue.len()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.priority_queue.is_empty()
    }

    /// Get the name of the current ordering strategy.
    pub fn strategy_name(&self) -> &'static str {
        self.strategy.name()
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, VarVec};
    use strata_identifiers::Buf32;
    use strata_ol_chain_types_new::{GamTxPayload, TransactionAttachment, TransactionPayload};

    use super::*;

    fn create_test_tx() -> OLTransaction {
        OLTransaction::new(
            TransactionAttachment::new_empty(),
            TransactionPayload::GenericAccountMessage(GamTxPayload::new(
                AccountId::new([0u8; 32]),
                VarVec::from_vec(vec![1, 2, 3]).unwrap(),
            )),
        )
    }

    fn create_test_metadata(entry_slot: u64) -> MempoolTxMetadata {
        MempoolTxMetadata {
            entry_slot,
            entry_time: 0,
            size_bytes: 100,
        }
    }

    #[test]
    fn test_fifo_ordering() {
        let strategy = FifoOrdering;
        let tx = create_test_tx();

        // Transaction entered at slot 101 (earlier)
        let meta_early = create_test_metadata(101);
        // Transaction entered at slot 102 (later)
        let meta_late = create_test_metadata(102);

        let prio_early = strategy.compute_priority(&tx, &meta_early);
        let prio_late = strategy.compute_priority(&tx, &meta_late);

        // Earlier slot should have higher priority
        assert!(prio_early > prio_late);
    }

    #[test]
    fn test_ordering_index_insert_and_retrieve() {
        let mut index = OrderingIndex::new(FifoOrdering);
        let tx = create_test_tx();

        let txid1 = OLTxId::from(Buf32::from([1u8; 32]));
        let txid2 = OLTxId::from(Buf32::from([2u8; 32]));
        let txid3 = OLTxId::from(Buf32::from([3u8; 32]));

        // Insert in non-priority order
        index.insert(txid2, &tx, &create_test_metadata(102));
        index.insert(txid1, &tx, &create_test_metadata(101));
        index.insert(txid3, &tx, &create_test_metadata(103));

        // Should retrieve in priority order (earliest first)
        let ordered = index.get_ordered_txids(10);
        assert_eq!(ordered.len(), 3);
        assert_eq!(ordered[0], txid1); // slot 101
        assert_eq!(ordered[1], txid2); // slot 102
        assert_eq!(ordered[2], txid3); // slot 103
    }

    #[test]
    fn test_ordering_index_remove() {
        let mut index = OrderingIndex::new(FifoOrdering);
        let tx = create_test_tx();

        let txid1 = OLTxId::from(Buf32::from([1u8; 32]));
        let txid2 = OLTxId::from(Buf32::from([2u8; 32]));

        index.insert(txid1, &tx, &create_test_metadata(101));
        index.insert(txid2, &tx, &create_test_metadata(102));

        assert_eq!(index.len(), 2);

        index.remove(&txid1);
        assert_eq!(index.len(), 1);

        let ordered = index.get_ordered_txids(10);
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0], txid2);
    }

    #[test]
    fn test_ordering_index_limit() {
        let mut index = OrderingIndex::new(FifoOrdering);
        let tx = create_test_tx();

        for i in 0..10 {
            let mut txid_bytes = [0u8; 32];
            txid_bytes[0] = i as u8;
            let txid = OLTxId::from(Buf32::from(txid_bytes));
            index.insert(txid, &tx, &create_test_metadata(i as u64 * 100 + 101));
        }

        assert_eq!(index.len(), 10);

        // Request only 5 transactions
        let ordered = index.get_ordered_txids(5);
        assert_eq!(ordered.len(), 5);

        // Should get the 5 earliest (highest priority)
        for i in 0..5 {
            let mut expected_bytes = [0u8; 32];
            expected_bytes[0] = i as u8;
            assert_eq!(
                ordered[i as usize],
                OLTxId::from(Buf32::from(expected_bytes))
            );
        }
    }
}
