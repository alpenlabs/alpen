//! Transaction ordering strategies for the mempool.

use crate::types::{MempoolEntry, OLMempoolTxPayload};

/// Trait for ordering transactions in the mempool.
///
/// Different strategies can be implemented (FIFO, fee-based, account-based, etc.)
/// to determine transaction priority.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "will be used in state management")
)]
pub(crate) trait OrderingStrategy: Send + Sync + 'static {
    /// Compute the priority value for a transaction.
    ///
    /// Lower values = higher priority (will be selected first).
    /// The priority is used to order transactions when retrieving them.
    ///
    /// Priority is a u128 value encoding both primary sort key (seq_no or slot)
    /// and insertion_id for deterministic tie-breaking.
    fn compute_priority(&self, entry: &MempoolEntry) -> u128;
}

/// FIFO (First-In-First-Out) ordering strategy.
///
/// For SnarkAccountUpdate transactions: Orders by sequence number (lower seq_no = higher priority).
/// For GenericAccountMessage transactions: Orders by `first_seen_slot`.
///
/// Priority encoding uses u128 bit-packing to avoid collisions:
/// - Upper 64 bits: Primary sort key (seq_no or first_seen_slot)
/// - Lower 64 bits: insertion_id (for deterministic tie-breaking)
///
/// This ensures unique priorities and FIFO ordering within same primary key.
#[derive(Debug, Clone, Default)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "will be used in state management")
)]
pub(crate) struct FifoOrderingStrategy;

impl OrderingStrategy for FifoOrderingStrategy {
    fn compute_priority(&self, entry: &MempoolEntry) -> u128 {
        match entry.tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => {
                // Primary: seq_no (lower seq_no = higher priority)
                // Tiebreaker: insertion_id (earlier insertion = higher priority)
                let seq_no = payload.base_update.operation.seq_no() as u128;
                let insertion_id = entry.ordering_key.insertion_id as u128;
                (seq_no << 64) | insertion_id
            }
            OLMempoolTxPayload::GenericAccountMessage(_) => {
                // Primary: first_seen_slot (earlier slot = higher priority)
                // Tiebreaker: insertion_id (earlier insertion = higher priority)
                let slot = entry.ordering_key.first_seen_slot as u128;
                let insertion_id = entry.ordering_key.insertion_id as u128;
                (slot << 64) | insertion_id
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        test_utils::{create_test_generic_tx, create_test_snark_tx},
        types::MempoolOrderingKey,
    };

    #[test]
    fn test_fifo_ordering() {
        let strategy = FifoOrderingStrategy;

        // Use GenericAccountMessage to test first_seen_slot ordering
        let tx = create_test_generic_tx();

        // Earlier slot should have higher priority (lower value)
        // Transactions in same slot have different insertion_ids
        let entry_slot100_id1 = MempoolEntry::new(tx.clone(), MempoolOrderingKey::new(100, 1), 100);
        let entry_slot100_id2 = MempoolEntry::new(tx.clone(), MempoolOrderingKey::new(100, 2), 100);
        let entry_slot200_id3 = MempoolEntry::new(tx, MempoolOrderingKey::new(200, 3), 100);

        let priority_100_1 = strategy.compute_priority(&entry_slot100_id1);
        let priority_100_2 = strategy.compute_priority(&entry_slot100_id2);
        let priority_200 = strategy.compute_priority(&entry_slot200_id3);

        // Same slot but different insertion_id = different priority (no collision)
        assert_ne!(priority_100_1, priority_100_2);
        // Earlier insertion_id has higher priority (lower value)
        assert!(priority_100_1 < priority_100_2);
        // Earlier slot has higher priority than later slot
        assert!(priority_100_1 < priority_200);
        assert!(priority_100_2 < priority_200);
    }

    #[test]
    fn test_fifo_ordering_edge_cases() {
        let strategy = FifoOrderingStrategy;

        // Use GenericAccountMessage to test first_seen_slot ordering
        let tx = create_test_generic_tx();

        // Slot 0 should have highest priority (lower value)
        let entry_0 = MempoolEntry::new(tx.clone(), MempoolOrderingKey::new(0, 1), 100);
        let entry_1 = MempoolEntry::new(tx.clone(), MempoolOrderingKey::new(1, 2), 100);
        let priority_0 = strategy.compute_priority(&entry_0);
        let priority_1 = strategy.compute_priority(&entry_1);
        assert!(priority_0 < priority_1);

        // Very large slots should still work
        let entry_max = MempoolEntry::new(tx, MempoolOrderingKey::new(u64::MAX, 3), 100);
        let priority_max = strategy.compute_priority(&entry_max);
        // Priority encodes both slot (upper 64 bits) and insertion_id (lower 64 bits)
        let expected = ((u64::MAX as u128) << 64) | 3u128;
        assert_eq!(priority_max, expected);
    }

    #[test]
    fn test_ordering_strategy_with_seq_no() {
        let strategy = FifoOrderingStrategy;

        // Create three test transactions with unique insertion_ids
        let mut map: BTreeMap<u128, u64> = BTreeMap::new();

        for i in 0..3 {
            let tx = create_test_snark_tx();
            let entry = MempoolEntry::new(tx.clone(), MempoolOrderingKey::new(100, i), 100);
            let priority = strategy.compute_priority(&entry);

            // Get seq_no from transaction
            let seq_no = match tx.payload() {
                OLMempoolTxPayload::SnarkAccountUpdate(payload) => {
                    payload.base_update.operation.seq_no()
                }
                _ => panic!("Expected SnarkAccountUpdate"),
            };

            map.insert(priority, seq_no);
        }

        let seq_nos: Vec<u64> = map.values().copied().collect();

        // Verify that lower seq_no has higher priority (ascending order)
        let sorted_by_seq: Vec<u64> = {
            let mut v = seq_nos.clone();
            v.sort();
            v
        };
        assert_eq!(seq_nos, sorted_by_seq, "Should give ascending seq_no order");
    }
}
