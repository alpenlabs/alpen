//! Core mempool types.

use std::{cmp::Ordering, collections::BTreeMap};

use strata_acct_types::AccountId;
pub use strata_ol_chain_types_new::OLTransaction;
use strata_ol_chain_types_new::TransactionPayload;

use crate::error::OLMempoolError;

/// Default maximum number of transactions in the mempool.
pub const DEFAULT_MAX_TX_COUNT: usize = 10_000;

/// Default maximum size of a single transaction in bytes.
pub const DEFAULT_MAX_TX_SIZE: usize = 1024 * 1024; // 1 MB

/// Default maximum total size of all transactions in mempool (bytes).
pub const DEFAULT_MAX_MEMPOOL_BYTES: usize = 1024 * 1024 * 1024; // 1 GB

/// Default maximum reorg depth for finding common ancestor.
/// OL chain doesn't expect reorgs, so this is a safety limit.
pub const DEFAULT_MAX_REORG_DEPTH: u64 = 50;

/// Default command channel buffer size.
pub const DEFAULT_COMMAND_BUFFER_SIZE: usize = 1000;

/// Configuration for the OL mempool.
#[derive(Clone, Debug)]
pub struct OLMempoolConfig {
    /// Maximum number of transactions in the mempool.
    pub max_tx_count: usize,

    /// Maximum size of a single transaction in bytes.
    pub max_tx_size: usize,

    /// Maximum total size of all transactions in mempool (bytes).
    pub max_mempool_bytes: usize,

    /// Maximum reorg depth for finding common ancestor during reorg handling.
    /// OL chain doesn't expect reorgs, so this is a safety limit to prevent infinite loops.
    pub max_reorg_depth: u64,

    /// Command channel buffer size.
    pub command_buffer_size: usize,
}

impl Default for OLMempoolConfig {
    fn default() -> Self {
        Self {
            max_tx_count: DEFAULT_MAX_TX_COUNT,
            max_tx_size: DEFAULT_MAX_TX_SIZE,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
            command_buffer_size: DEFAULT_COMMAND_BUFFER_SIZE,
        }
    }
}

/// Ordering key for mempool transactions.
///
/// Provides collision-free ordering with different strategies for different transaction types:
/// - **Snark transactions**: Strict per-account seq_no ordering with FIFO tiebreaking across
///   accounts
/// - **GAM transactions**: Pure FIFO ordering by `timestamp_micros`
///
/// The `timestamp_micros` is in microseconds since UNIX epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MempoolOrderingKey {
    /// Snark account update transaction.
    ///
    /// Ordered by:
    /// 1. Within same account: seq_no (strict ordering)
    /// 2. Across accounts: timestamp (FIFO)
    Snark {
        /// Target account ID.
        account_id: AccountId,
        /// Sequence number for this account (from SnarkAccountUpdate).
        seq_no: u64,
        /// Timestamp (microseconds since UNIX epoch) for cross-account FIFO ordering.
        timestamp_micros: u64,
    },

    /// Generic account message transaction.
    ///
    /// Ordered by timestamp only (pure FIFO).
    Gam {
        /// Timestamp (microseconds since UNIX epoch) for FIFO ordering.
        timestamp_micros: u64,
    },
}

impl MempoolOrderingKey {
    /// Create ordering key for a transaction with the given timestamp_micros.
    pub(crate) fn for_transaction(tx: &OLTransaction, timestamp_micros: u64) -> Self {
        match tx.payload() {
            TransactionPayload::SnarkAccountUpdate(payload) => Self::Snark {
                account_id: *payload.target(),
                seq_no: payload.operation().update().seq_no(),
                timestamp_micros,
            },
            TransactionPayload::GenericAccountMessage(_) => Self::Gam { timestamp_micros },
        }
    }

    /// Get the timestamp from this ordering key.
    pub fn timestamp_micros(&self) -> u64 {
        match self {
            Self::Snark {
                timestamp_micros, ..
            } => *timestamp_micros,
            Self::Gam { timestamp_micros } => *timestamp_micros,
        }
    }
}

impl PartialOrd for MempoolOrderingKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MempoolOrderingKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            // Both Snark: same account? order by seq_no, else by `timestamp_micros`
            (
                Self::Snark {
                    account_id: a1,
                    seq_no: s1,
                    timestamp_micros: t1,
                },
                Self::Snark {
                    account_id: a2,
                    seq_no: s2,
                    timestamp_micros: t2,
                },
            ) => {
                if a1 == a2 {
                    s1.cmp(s2)
                } else {
                    t1.cmp(t2)
                }
            }

            // Both GAM: order by `timestamp_micros`
            (
                Self::Gam {
                    timestamp_micros: t1,
                },
                Self::Gam {
                    timestamp_micros: t2,
                },
            ) => t1.cmp(t2),

            // Mixed Snark/GAM: use `timestamp_micros` for fair interleaving
            (
                Self::Snark {
                    timestamp_micros, ..
                },
                Self::Gam {
                    timestamp_micros: t2,
                },
            )
            | (
                Self::Gam { timestamp_micros },
                Self::Snark {
                    timestamp_micros: t2,
                    ..
                },
            ) => timestamp_micros.cmp(t2),
        }
    }
}

/// Internal mempool entry combining transaction data with ordering metadata.
///
/// This is used internally by the mempool implementation and not exposed in the public API.
#[derive(Clone, Debug)]
pub(crate) struct MempoolEntry {
    /// The transaction data.
    pub(crate) tx: OLTransaction,

    /// Ordering key.
    pub(crate) ordering_key: MempoolOrderingKey,

    /// Size of the transaction in bytes (for capacity management).
    pub(crate) size_bytes: usize,
}

impl MempoolEntry {
    /// Create a new mempool entry.
    pub(crate) fn new(
        tx: OLTransaction,
        ordering_key: MempoolOrderingKey,
        size_bytes: usize,
    ) -> Self {
        Self {
            tx,
            ordering_key,
            size_bytes,
        }
    }
}

/// Statistics about the mempool state.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct OLMempoolStats {
    /// Current number of transactions in the mempool.
    pub(crate) mempool_size: usize,

    /// Total size of all transactions in bytes.
    pub(crate) total_bytes: usize,

    /// Total enqueued transactions (accepted).
    pub(crate) enqueues_accepted: u64,

    /// Total rejected transactions.
    pub(crate) enqueues_rejected: u64,

    /// Rejections by reason.
    pub(crate) rejects_by_reason: OLMempoolRejectCounts,

    /// Total evictions due to capacity limits.
    pub(crate) evictions: u64,
}

impl OLMempoolStats {
    /// Create new mempool statistics.
    #[expect(dead_code, reason = "will be used in mempool implementation")]
    pub(crate) fn new() -> Self {
        Self {
            mempool_size: 0,
            total_bytes: 0,
            enqueues_accepted: 0,
            enqueues_rejected: 0,
            rejects_by_reason: OLMempoolRejectCounts::default(),
            evictions: 0,
        }
    }

    /// Get current mempool size.
    pub fn mempool_size(&self) -> usize {
        self.mempool_size
    }

    /// Get total bytes.
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Get accepted enqueues.
    pub fn enqueues_accepted(&self) -> u64 {
        self.enqueues_accepted
    }

    /// Get rejected enqueues.
    pub fn enqueues_rejected(&self) -> u64 {
        self.enqueues_rejected
    }

    /// Get reject reasons.
    pub fn rejects_by_reason(&self) -> &OLMempoolRejectCounts {
        &self.rejects_by_reason
    }

    /// Get evictions count.
    pub fn evictions(&self) -> u64 {
        self.evictions
    }
}

/// Reason for rejecting a transaction from the mempool.
///
/// This represents the different types of rejections that can occur.
/// Note: This does not include non-rejection errors like Database or Serialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub enum OLMempoolRejectReason {
    /// Rejected due to mempool size limit exceeded.
    MempoolFull,

    /// Rejected due to target account not existing.
    AccountDoesNotExist,

    /// Rejected due to account type mismatch (e.g., SnarkAccountUpdate targeting non-Snark
    /// account).
    AccountTypeMismatch,

    /// Rejected due to transaction too large.
    TransactionTooLarge,

    /// Rejected due to already used sequence number.
    UsedSequenceNumber,

    /// Rejected due to sequence number gap (expected sequential order).
    SequenceNumberGap,

    /// Rejected due to expired (max_slot in past).
    TransactionExpired,

    /// Rejected due to not mature (min_slot in future).
    TransactionNotMature,

    /// Duplicate transaction (already in mempool).
    Duplicate,
}

impl OLMempoolRejectReason {
    /// Try to extract a rejection reason from an [`OLMempoolError`].
    ///
    /// Returns `Some(reason)` if the error represents a transaction rejection
    /// that should be tracked in statistics.
    ///
    /// Returns `None` for errors that are not rejection reasons:
    /// - Internal errors (Database, Serialization) - these are system errors, not rejections
    /// - Query errors (TransactionNotFound) - these are lookup failures, not rejections
    ///
    /// Note: Some rejection reasons (like `Duplicate`) are not errors and are tracked
    /// separately during idempotent submission.
    pub fn from_error(error: &OLMempoolError) -> Option<Self> {
        match error {
            OLMempoolError::MempoolFull { .. } => Some(Self::MempoolFull),
            OLMempoolError::MempoolByteLimitExceeded { .. } => Some(Self::MempoolFull),
            OLMempoolError::AccountDoesNotExist { .. } => Some(Self::AccountDoesNotExist),
            OLMempoolError::AccountTypeMismatch { .. } => Some(Self::AccountTypeMismatch),
            OLMempoolError::TransactionTooLarge { .. } => Some(Self::TransactionTooLarge),
            OLMempoolError::TransactionExpired { .. } => Some(Self::TransactionExpired),
            OLMempoolError::TransactionNotMature { .. } => Some(Self::TransactionNotMature),
            OLMempoolError::UsedSequenceNumber { .. } => Some(Self::UsedSequenceNumber),
            OLMempoolError::SequenceNumberGap { .. } => Some(Self::SequenceNumberGap),
            OLMempoolError::AccountStateAccess(_)
            | OLMempoolError::TransactionNotFound(_)
            | OLMempoolError::Database(_)
            | OLMempoolError::StateProvider(_)
            | OLMempoolError::Serialization(_)
            | OLMempoolError::ServiceClosed(_) => None,
        }
    }
}

/// Reason a transaction is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MempoolTxInvalidReason {
    /// Transaction is permanently invalid (consensus rules, expired).
    /// Will be removed from mempool.
    Invalid,

    /// Transaction failed (may succeed later or may be a transient infrastructure issue).
    /// Will stay in mempool until revalidation.
    Failed,
}

/// Breakdown of rejection counts for statistics.
///
/// Uses a [`BTreeMap`] to track counts per [`OLMempoolRejectReason`], making it easy to
/// iterate, extend, and work with programmatically.
///
/// See [`OLMempoolRejectReason::from_error`] for converting errors to rejection reasons.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct OLMempoolRejectCounts {
    counts: BTreeMap<OLMempoolRejectReason, u64>,
}

impl OLMempoolRejectCounts {
    /// Increment the count for a given rejection reason.
    pub fn increment(&mut self, reason: OLMempoolRejectReason) {
        *self.counts.entry(reason).or_insert(0) += 1;
    }

    /// Get the count for a specific rejection reason.
    pub fn get(&self, reason: OLMempoolRejectReason) -> u64 {
        self.counts.get(&reason).copied().unwrap_or(0)
    }

    /// Get all rejection reason counts as an iterator.
    pub fn iter(&self) -> impl Iterator<Item = (OLMempoolRejectReason, u64)> + '_ {
        self.counts.iter().map(|(k, v)| (*k, *v))
    }

    /// Get the total count of all rejections.
    pub fn total(&self) -> u64 {
        self.counts.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use ::ssz::{Decode, Encode};
    use proptest::{
        prelude::*,
        strategy::{Strategy, ValueTree},
        test_runner::TestRunner,
    };
    use strata_acct_types::AccountId;
    use strata_ol_chain_types_new::{
        OLTransaction, OLTransactionData, TransactionPayload, TxProofs, test_utils,
    };

    use super::*;
    use crate::test_utils::{
        create_test_account_id, create_test_constraints, create_test_generic_tx,
        create_test_snark_tx, create_test_snark_tx_from_update, create_test_snark_tx_with_seq_no,
        create_test_snark_update,
    };

    fn create_test_message_payload() -> Vec<u8> {
        let mut runner = TestRunner::default();
        prop::collection::vec(any::<u8>(), 1..64)
            .new_tree(&mut runner)
            .unwrap()
            .current()
    }

    #[test]
    fn test_generic_account_message_creation() {
        let target = create_test_account_id();
        let constraints = create_test_constraints();
        let payload = create_test_message_payload();

        let tx = OLTransaction::new(
            OLTransactionData::new_gam(target, payload).with_constraints(constraints.clone()),
            TxProofs::new_empty(),
        );

        assert_eq!(tx.target(), Some(target));
        assert_eq!(tx.constraints(), &constraints);
        assert!(matches!(
            tx.payload(),
            TransactionPayload::GenericAccountMessage(_)
        ));
    }

    #[test]
    fn test_snark_account_update_creation() {
        let target = create_test_account_id();
        let base_update = create_test_snark_update();
        let constraints = create_test_constraints();

        let tx = create_test_snark_tx_from_update(target, base_update.clone(), constraints.clone());

        assert_eq!(tx.target(), Some(target));
        assert_eq!(tx.constraints(), &constraints);
        match tx.payload() {
            TransactionPayload::SnarkAccountUpdate(payload) => {
                assert_eq!(payload.target(), &target);
                assert_eq!(
                    payload.operation().update().seq_no(),
                    base_update.operation().seq_no()
                );
            }
            _ => panic!("Expected SnarkAccountUpdate"),
        }
    }

    #[test]
    fn test_compute_txid_generic_message() {
        let target = create_test_account_id();
        let payload = create_test_message_payload();
        let tx1 = OLTransaction::new(
            OLTransactionData::new_gam(target, payload.clone()),
            TxProofs::new_empty(),
        );
        let tx2 = OLTransaction::new(
            OLTransactionData::new_gam(target, payload),
            TxProofs::new_empty(),
        );
        assert_eq!(tx1.compute_txid(), tx2.compute_txid());

        let different_target = create_test_account_id();
        let tx3 = OLTransaction::new(
            OLTransactionData::new_gam(different_target, create_test_message_payload()),
            TxProofs::new_empty(),
        );
        assert_ne!(tx1.compute_txid(), tx3.compute_txid());
    }

    #[test]
    fn test_compute_txid_snark_update() {
        let tx1 = create_test_snark_tx();
        let tx2 = tx1.clone();
        assert_eq!(tx1.compute_txid(), tx2.compute_txid());

        let tx3 = create_test_snark_tx_with_seq_no(1, 777);
        assert_ne!(tx1.compute_txid(), tx3.compute_txid());
    }

    #[test]
    fn test_ssz_roundtrip_generic_message() {
        let tx = create_test_generic_tx();
        let encoded = Encode::as_ssz_bytes(&tx);
        let decoded = OLTransaction::from_ssz_bytes(&encoded).expect("Should decode");
        assert_eq!(tx, decoded);
    }

    #[test]
    fn test_ssz_roundtrip_snark_update() {
        let tx = create_test_snark_tx();
        let encoded = Encode::as_ssz_bytes(&tx);
        let decoded = OLTransaction::from_ssz_bytes(&encoded).expect("Should decode");
        assert_eq!(tx, decoded);
    }

    proptest! {
        #[test]
        fn test_mempool_tx_ssz_roundtrip(tx in test_utils::ol_transaction_strategy()) {
            let encoded = Encode::as_ssz_bytes(&tx);
            let decoded = OLTransaction::from_ssz_bytes(&encoded).expect("Should decode transaction");
            prop_assert_eq!(tx, decoded);
        }

        #[test]
        fn test_mempool_tx_id_consistency(tx in test_utils::ol_transaction_strategy()) {
            let txid1 = tx.compute_txid();
            let txid2 = tx.compute_txid();
            prop_assert_eq!(txid1, txid2, "Transaction ID should be deterministic");
        }

        #[test]
        fn test_mempool_tx_payload_has_target(tx in test_utils::ol_transaction_strategy()) {
            let target = tx.target().expect("all payload variants must have target");
            prop_assert!(target != AccountId::zero(), "Target should not be zero");
        }
    }

    // Tests for MempoolOrderingKey::Ord implementation
    mod ordering_tests {
        use std::cmp::Ordering;

        use super::*;
        use crate::test_utils::{
            create_test_account_id, create_test_generic_tx, create_test_snark_tx,
        };

        #[test]
        fn test_gam_ordering_by_timestamp_micros() {
            let tx = create_test_generic_tx();
            let entries: Vec<_> = (1..=3)
                .map(|ts| {
                    MempoolEntry::new(
                        tx.clone(),
                        MempoolOrderingKey::Gam {
                            timestamp_micros: ts,
                        },
                        100,
                    )
                })
                .collect();

            for i in 0..entries.len() - 1 {
                assert_eq!(
                    entries[i].ordering_key.cmp(&entries[i + 1].ordering_key),
                    Ordering::Less
                );
            }
        }

        #[test]
        fn test_snark_same_account_orders_by_seq_no() {
            let account = create_test_account_id();
            let tx = create_test_snark_tx();
            let timestamps = [1_000_100, 1_000_050, 1_000_025]; // Decreasing timestamps
            let entries: Vec<_> = timestamps
                .iter()
                .enumerate()
                .map(|(i, &ts)| {
                    MempoolEntry::new(
                        tx.clone(),
                        MempoolOrderingKey::Snark {
                            account_id: account,
                            seq_no: i as u64 + 1,
                            timestamp_micros: ts,
                        },
                        100,
                    )
                })
                .collect();

            // Same account: seq_no determines order regardless of timestamp
            for i in 0..entries.len() - 1 {
                assert_eq!(
                    entries[i].ordering_key.cmp(&entries[i + 1].ordering_key),
                    Ordering::Less
                );
            }
        }

        #[test]
        fn test_snark_different_accounts_orders_by_timestamp_micros() {
            let account_a = create_test_account_id();
            let mut account_b = create_test_account_id();
            while account_b == account_a {
                account_b = create_test_account_id();
            }

            // Different accounts - should order by `timestamp_micros`
            let tx_a = create_test_snark_tx();
            let tx_b = create_test_snark_tx();

            // Lower seq_no, later timestamp
            let entry_a = MempoolEntry::new(
                tx_a,
                MempoolOrderingKey::Snark {
                    account_id: account_a,
                    seq_no: 5,
                    timestamp_micros: 2_000_100,
                },
                100,
            );
            // Higher seq_no, earlier timestamp
            let entry_b = MempoolEntry::new(
                tx_b,
                MempoolOrderingKey::Snark {
                    account_id: account_b,
                    seq_no: 7,
                    timestamp_micros: 1_000_050,
                },
                100,
            );

            // Different accounts: `timestamp_micros` determines order
            assert_eq!(
                entry_b.ordering_key.cmp(&entry_a.ordering_key),
                Ordering::Less
            );
        }

        #[test]
        fn test_mixed_snark_gam_orders_by_timestamp_micros() {
            let account = create_test_account_id();

            let tx_snark = create_test_snark_tx();
            let tx_gam = create_test_generic_tx();

            // Snark with earlier `timestamp_micros` should come first
            let entry_snark = MempoolEntry::new(
                tx_snark,
                MempoolOrderingKey::Snark {
                    account_id: account,
                    seq_no: 1,
                    timestamp_micros: 1_000_050,
                },
                100,
            );
            let entry_gam = MempoolEntry::new(
                tx_gam,
                MempoolOrderingKey::Gam {
                    timestamp_micros: 2_000_100,
                },
                100,
            );

            // Mixed: `timestamp_micros` determines order
            assert_eq!(
                entry_snark.ordering_key.cmp(&entry_gam.ordering_key),
                Ordering::Less
            );
        }

        #[test]
        fn test_complex_ordering_scenario() {
            let acc_a = create_test_account_id();
            let mut acc_b = create_test_account_id();
            while acc_b == acc_a {
                acc_b = create_test_account_id();
            }
            let (tx_gam, tx_snark) = (create_test_generic_tx(), create_test_snark_tx());

            let mut entries = [
                MempoolEntry::new(
                    tx_gam.clone(),
                    MempoolOrderingKey::Gam {
                        timestamp_micros: 1_000_010,
                    },
                    100,
                ),
                MempoolEntry::new(
                    tx_snark.clone(),
                    MempoolOrderingKey::Snark {
                        account_id: acc_a,
                        seq_no: 1,
                        timestamp_micros: 1_000_020,
                    },
                    100,
                ),
                MempoolEntry::new(
                    tx_gam,
                    MempoolOrderingKey::Gam {
                        timestamp_micros: 1_000_030,
                    },
                    100,
                ),
                MempoolEntry::new(
                    tx_snark.clone(),
                    MempoolOrderingKey::Snark {
                        account_id: acc_a,
                        seq_no: 2,
                        timestamp_micros: 1_000_040,
                    },
                    100,
                ),
                MempoolEntry::new(
                    tx_snark,
                    MempoolOrderingKey::Snark {
                        account_id: acc_b,
                        seq_no: 1,
                        timestamp_micros: 1_000_050,
                    },
                    100,
                ),
            ];

            entries.sort_by(|a, b| a.ordering_key.cmp(&b.ordering_key));
            let timestamps: Vec<u64> = entries
                .iter()
                .map(|e| e.ordering_key.timestamp_micros())
                .collect();
            assert_eq!(
                timestamps,
                vec![1_000_010, 1_000_020, 1_000_030, 1_000_040, 1_000_050]
            );
        }
    }
}
