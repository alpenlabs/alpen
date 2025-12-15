//! Iterator for best transactions with mark_invalid callback
//!
//! Provides Reth-style iterator pattern for block assembly.

use std::collections::HashSet;

use strata_acct_types::AccountId;
use strata_identifiers::OLTxId;

use crate::types::OLMempoolTransaction;

/// Trait for iterating over best transactions with ability to mark invalid
///
/// Similar to `reth_transaction_pool::BestTransactions` but simplified for OL.
/// Block assembly iterates over transactions and marks failed ones for removal.
pub trait BestTransactions: Iterator<Item = (OLTxId, OLMempoolTransaction)> {
    /// Mark transaction as invalid for removal
    ///
    /// Queues the transaction ID for removal. Caller must retrieve marked
    /// transactions via `marked_invalid()` and remove them from mempool.
    fn mark_invalid(&mut self, txid: OLTxId);

    /// Get marked transaction IDs without consuming iterator
    ///
    /// Returns the set of transaction IDs marked as invalid during iteration.
    /// Caller should remove these from mempool after iteration completes.
    fn marked_invalid(&self) -> Vec<OLTxId>;
}

/// Iterator over best transactions in priority order
///
/// # Usage
/// Block assembly iterates over transactions and marks failed ones using
/// `mark_invalid()`. After iteration completes, the mempool service retrieves
/// marked transactions via `into_marked_invalid()` and removes them.
#[derive(Debug)]
pub struct BestTransactionsIterator {
    /// Transactions to iterate over (txid, transaction) in priority order
    transactions: Vec<(OLTxId, OLMempoolTransaction)>,

    /// Current position in iteration
    position: usize,

    /// Transaction IDs marked as invalid during iteration
    marked_invalid: HashSet<OLTxId>,
}

impl BestTransactionsIterator {
    /// Create new iterator from ordered transactions
    ///
    /// # Arguments
    /// * `transactions` - Transactions in priority order (highest first)
    pub fn new(transactions: Vec<(OLTxId, OLMempoolTransaction)>) -> Self {
        Self {
            transactions,
            position: 0,
            marked_invalid: HashSet::new(),
        }
    }

    /// Get all transaction IDs marked as invalid
    ///
    /// Used by mempool service to remove marked transactions on drop.
    pub fn into_marked_invalid(self) -> HashSet<OLTxId> {
        self.marked_invalid
    }

    /// Get marked transaction IDs without consuming iterator.
    ///
    /// Used by block assembly to retrieve marked transactions for cleanup.
    /// After iteration completes, caller should remove these from mempool.
    pub fn marked_invalid(&self) -> Vec<OLTxId> {
        self.marked_invalid.iter().copied().collect()
    }

    /// Find account and sequence number for a transaction
    ///
    /// Used by `mark_invalid` to identify dependent transactions.
    fn find_account_and_seq_no(&self, txid: &OLTxId) -> Option<(AccountId, u64)> {
        self.transactions
            .iter()
            .find(|(id, _)| id == txid)
            .and_then(|(_, tx)| {
                tx.base_update()
                    .map(|u| (tx.target(), u.operation().seq_no()))
            })
    }
}

impl Iterator for BestTransactionsIterator {
    type Item = (OLTxId, OLMempoolTransaction);

    fn next(&mut self) -> Option<Self::Item> {
        while self.position < self.transactions.len() {
            let item = self.transactions[self.position].clone();
            self.position += 1;

            // Skip marked transactions
            if !self.marked_invalid.contains(&item.0) {
                return Some(item);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.transactions.len().saturating_sub(self.position);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for BestTransactionsIterator {
    fn len(&self) -> usize {
        self.transactions.len().saturating_sub(self.position)
    }
}

impl BestTransactions for BestTransactionsIterator {
    fn mark_invalid(&mut self, txid: OLTxId) {
        self.marked_invalid.insert(txid);

        // Defensive: Mark higher seq_nos from same account
        if let Some((account, seq_no)) = self.find_account_and_seq_no(&txid) {
            for i in self.position..self.transactions.len() {
                let (other_txid, other_tx) = &self.transactions[i];
                if let Some(other_update) = other_tx.base_update()
                    && other_tx.target() == account
                    && other_update.operation().seq_no() > seq_no
                {
                    self.marked_invalid.insert(*other_txid);
                }
            }
        }
    }

    fn marked_invalid(&self) -> Vec<OLTxId> {
        self.marked_invalid.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_test_generic_tx;

    fn create_test_tx() -> (OLTxId, OLMempoolTransaction) {
        let tx = create_test_generic_tx();
        let txid = tx.compute_txid();
        (txid, tx)
    }

    #[test]
    fn test_iterator_basic() {
        let txs = vec![create_test_tx(), create_test_tx(), create_test_tx()];
        let mut iter = BestTransactionsIterator::new(txs.clone());

        // Iterate and collect
        let collected: Vec<_> = iter.by_ref().collect();
        assert_eq!(collected.len(), 3);
        assert_eq!(collected[0].0, txs[0].0);
        assert_eq!(collected[1].0, txs[1].0);
        assert_eq!(collected[2].0, txs[2].0);
    }

    #[test]
    fn test_mark_invalid() {
        let txs = vec![create_test_tx(), create_test_tx(), create_test_tx()];
        let mut iter = BestTransactionsIterator::new(txs.clone());

        // Mark second transaction as invalid
        iter.mark_invalid(txs[1].0);

        // Marked transaction should be in set
        let marked = iter.into_marked_invalid();
        assert_eq!(marked.len(), 1);
        assert!(marked.contains(&txs[1].0));
    }

    #[test]
    fn test_mark_invalid_during_iteration() {
        let txs = vec![create_test_tx(), create_test_tx(), create_test_tx()];
        let txids = [txs[0].0, txs[1].0, txs[2].0];
        let mut iter = BestTransactionsIterator::new(txs);

        // Iterate and mark some as invalid
        let mut count = 0;
        while let Some((txid, _tx)) = iter.next() {
            count += 1;
            if count == 2 {
                iter.mark_invalid(txid); // Mark second one
            }
        }

        assert_eq!(count, 3);
        let marked = iter.into_marked_invalid();
        assert_eq!(marked.len(), 1);
        assert!(marked.contains(&txids[1]));
    }

    #[test]
    fn test_exact_size_iterator() {
        let txs = vec![create_test_tx(), create_test_tx(), create_test_tx()];
        let mut iter = BestTransactionsIterator::new(txs);

        assert_eq!(iter.len(), 3);
        iter.next();
        assert_eq!(iter.len(), 2);
        iter.next();
        assert_eq!(iter.len(), 1);
        iter.next();
        assert_eq!(iter.len(), 0);
    }

    #[test]
    fn test_block_assembly_use_case() {
        // Simulate block assembly workflow
        let txs = vec![create_test_tx(), create_test_tx(), create_test_tx()];
        let tx1_id = txs[0].0;
        let tx2_id = txs[1].0;
        let tx3_id = txs[2].0;

        let mut iter = BestTransactionsIterator::new(txs);

        // Block assembly iterates and validates
        let mut included = vec![];
        while let Some((txid, _tx)) = iter.next() {
            // Simulate validation
            if txid == tx2_id {
                // Simulate validation failure
                iter.mark_invalid(txid);
            } else {
                // Simulate success - would include in block
                included.push(txid);
            }
        }

        // After iteration, get marked transactions for removal
        let marked = iter.into_marked_invalid();

        // Verify: tx1 and tx3 included, tx2 marked for removal
        assert_eq!(included.len(), 2);
        assert!(included.contains(&tx1_id));
        assert!(included.contains(&tx3_id));
        assert_eq!(marked.len(), 1);
        assert!(marked.contains(&tx2_id));

        // Mempool service would now call remove_transactions(&marked)
    }

    #[test]
    fn test_iterator_from_handle_result() {
        // Test that the Vec returned by handle.best_transactions() can be wrapped in iterator
        let txs = vec![create_test_tx(), create_test_tx(), create_test_tx()];
        let txids: Vec<_> = txs.iter().map(|(id, _)| *id).collect();

        // Simulate what handle.best_transactions() returns
        let vec_result: Vec<(OLTxId, OLMempoolTransaction)> = txs;

        // Wrap in iterator (as block assembly would do)
        let mut iter = BestTransactionsIterator::new(vec_result);

        // Iterate and mark some as invalid
        let mut collected = vec![];
        while let Some((txid, _tx)) = iter.next() {
            if txid == txids[1] {
                iter.mark_invalid(txid);
            } else {
                collected.push(txid);
            }
        }

        // Verify iteration worked
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0], txids[0]);
        assert_eq!(collected[1], txids[2]);

        // Verify marked transactions
        let marked = iter.into_marked_invalid();
        assert_eq!(marked.len(), 1);
        assert!(marked.contains(&txids[1]));
    }

    #[test]
    fn test_iterator_skips_marked_transactions() {
        use crate::test_utils::create_test_snark_tx_with_seq_no;

        // Create transactions from different accounts to avoid dependency marking
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(2, 0);
        let tx2 = create_test_snark_tx_with_seq_no(3, 0);

        let txid0 = tx0.compute_txid();
        let txid1 = tx1.compute_txid();
        let txid2 = tx2.compute_txid();

        let txs = vec![(txid0, tx0), (txid1, tx1.clone()), (txid2, tx2)];

        let mut iter = BestTransactionsIterator::new(txs);

        // Mark tx1 as invalid before iteration
        iter.mark_invalid(txid1);

        // Iterate - should skip tx1
        let mut collected = vec![];
        for (txid, _tx) in iter.by_ref() {
            collected.push(txid);
        }

        // Should only see tx0 and tx2
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0], txid0);
        assert_eq!(collected[1], txid2);

        // Marked set should contain tx1
        let marked = iter.into_marked_invalid();
        assert_eq!(marked.len(), 1);
        assert!(marked.contains(&txid1));
    }

    #[test]
    fn test_mark_invalid_marks_dependent_transactions() {
        use crate::test_utils::create_test_snark_tx_with_seq_no;

        // Create transactions with same account, sequential seq_nos
        let tx0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx2 = create_test_snark_tx_with_seq_no(1, 2);
        let tx3 = create_test_snark_tx_with_seq_no(1, 3);

        let txid0 = tx0.compute_txid();
        let txid1 = tx1.compute_txid();
        let txid2 = tx2.compute_txid();
        let txid3 = tx3.compute_txid();

        let txs = vec![
            (txid0, tx0.clone()),
            (txid1, tx1.clone()),
            (txid2, tx2.clone()),
            (txid3, tx3.clone()),
        ];

        let mut iter = BestTransactionsIterator::new(txs);

        // Iterate and include tx0
        let (first_txid, _) = iter.next().unwrap();
        assert_eq!(first_txid, txid0);

        // Iterate and get tx1, but mark it invalid
        let (second_txid, _) = iter.next().unwrap();
        assert_eq!(second_txid, txid1);
        iter.mark_invalid(second_txid);

        // Rest of iteration should skip tx2 and tx3 (dependent on failed tx1)
        let rest: Vec<_> = iter.by_ref().collect();
        assert_eq!(rest.len(), 0);

        // Marked set should contain tx1, tx2, tx3 (all dependent transactions)
        let marked = iter.into_marked_invalid();
        assert_eq!(marked.len(), 3);
        assert!(marked.contains(&txid1));
        assert!(marked.contains(&txid2));
        assert!(marked.contains(&txid3));
    }

    #[test]
    fn test_mark_invalid_only_marks_same_account() {
        use crate::test_utils::create_test_snark_tx_with_seq_no;

        // Create transactions from different accounts
        let tx_acc1_0 = create_test_snark_tx_with_seq_no(1, 0);
        let tx_acc1_1 = create_test_snark_tx_with_seq_no(1, 1);
        let tx_acc2_0 = create_test_snark_tx_with_seq_no(2, 0);
        let tx_acc2_1 = create_test_snark_tx_with_seq_no(2, 1);

        let txid_acc1_0 = tx_acc1_0.compute_txid();
        let txid_acc1_1 = tx_acc1_1.compute_txid();
        let txid_acc2_0 = tx_acc2_0.compute_txid();
        let txid_acc2_1 = tx_acc2_1.compute_txid();

        let txs = vec![
            (txid_acc1_0, tx_acc1_0),
            (txid_acc1_1, tx_acc1_1),
            (txid_acc2_0, tx_acc2_0),
            (txid_acc2_1, tx_acc2_1),
        ];

        let mut iter = BestTransactionsIterator::new(txs);

        // Mark acc1's first transaction invalid
        let (first_txid, _) = iter.next().unwrap();
        assert_eq!(first_txid, txid_acc1_0);
        iter.mark_invalid(first_txid);

        // Should skip acc1_1 but include acc2_0 and acc2_1
        let rest: Vec<_> = iter.by_ref().map(|(txid, _)| txid).collect();
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0], txid_acc2_0);
        assert_eq!(rest[1], txid_acc2_1);

        // Marked set should only contain acc1 transactions
        let marked = iter.into_marked_invalid();
        assert_eq!(marked.len(), 2);
        assert!(marked.contains(&txid_acc1_0));
        assert!(marked.contains(&txid_acc1_1));
        assert!(!marked.contains(&txid_acc2_0));
        assert!(!marked.contains(&txid_acc2_1));
    }
}
