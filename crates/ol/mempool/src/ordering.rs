//! Mempool ordering policies and ordering keys.

use std::fmt::Debug;

use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

/// Policy trait for computing mempool transaction priorities.
///
/// Implementations define how priorities are computed from transaction data and insertion
/// metadata. Priority ordering is interpreted with the invariant that iterating in ascending order
/// yields highest-priority transactions first.
pub trait MempoolPriorityPolicy: Clone + Copy + Debug + Send + Sync + 'static {
    /// Ordering priority used by the policy.
    type Priority: Ord + Copy + Debug + Send + Sync + 'static;

    /// Compute an ordering priority for a transaction.
    fn compute_priority(tx: &OLTransaction, timestamp_micros: u64) -> Self::Priority;
}

/// FIFO priority policy.
///
/// This is the current default behavior and will continue to be used unless another policy is
/// explicitly selected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FifoPriority;

impl MempoolPriorityPolicy for FifoPriority {
    type Priority = FifoPriorityKey;

    fn compute_priority(tx: &OLTransaction, timestamp_micros: u64) -> Self::Priority {
        FifoPriorityKey::for_tx(timestamp_micros, tx)
    }
}

/// FIFO package-ordering key.
///
/// This key is only for global package ordering and does not encode intra-package dependencies.
/// The `timestamp_micros` is in microseconds since UNIX epoch.
///
/// Derived [`Ord`] is load-bearing here: field declaration order defines sort order.
/// `timestamp_micros` is first and `txid` is second (tie-breaker); do not reorder fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FifoPriorityKey {
    timestamp_micros: u64,
    txid: OLTxId,
}

impl FifoPriorityKey {
    /// Creates a FIFO priority key from `timestamp_micros` and `txid`.
    pub(crate) fn new(timestamp_micros: u64, txid: OLTxId) -> Self {
        Self {
            timestamp_micros,
            txid,
        }
    }

    /// Create ordering key for a transaction.
    pub(crate) fn for_tx(timestamp_micros: u64, tx: &OLTransaction) -> Self {
        Self::new(timestamp_micros, tx.compute_txid())
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;
    use crate::{
        test_utils::{
            create_test_generic_tx, create_test_snark_tx_with_seq_no, create_test_txid_with,
        },
        types::MempoolEntry,
    };

    #[test]
    fn test_priority_orders_by_timestamp_micros() {
        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let entries: Vec<_> = (1..=3)
            .map(|ts| {
                MempoolEntry::<FifoPriority>::new(
                    tx.clone(),
                    FifoPriorityKey {
                        timestamp_micros: ts,
                        txid: create_test_txid_with(ts as u8),
                    },
                    100,
                )
            })
            .collect();

        for i in 0..entries.len() - 1 {
            assert_eq!(
                entries[i].priority.cmp(&entries[i + 1].priority),
                Ordering::Less
            );
        }
    }

    #[test]
    fn test_priority_tie_breaks_with_txid() {
        let tx = create_test_generic_tx();
        let key_a = FifoPriorityKey::for_tx(1_000_000, &tx);
        let key_b = FifoPriorityKey {
            timestamp_micros: key_a.timestamp_micros,
            txid: create_test_txid_with(255),
        };
        assert!(key_a != key_b);
        assert_ne!(key_a.cmp(&key_b), Ordering::Equal);
    }

    #[test]
    fn test_priority_tie_break_uses_txid() {
        let key_a = FifoPriorityKey {
            timestamp_micros: 99,
            txid: create_test_txid_with(1),
        };
        let key_b = FifoPriorityKey {
            timestamp_micros: 99,
            txid: create_test_txid_with(2),
        };
        assert_eq!(key_a.cmp(&key_b), Ordering::Less);
    }
}
