//! Core mempool types.

use std::collections::BTreeMap;

pub use strata_ol_chain_types_new::OLTransaction;

use crate::{error::OLMempoolError, ordering::MempoolPriorityPolicy};

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

/// Internal mempool entry combining transaction data with ordering metadata.
#[derive(Clone, Debug)]
pub(crate) struct MempoolEntry<P: MempoolPriorityPolicy> {
    /// The transaction data.
    pub(crate) tx: OLTransaction,

    /// Ordering key.
    pub(crate) priority: P::Priority,

    /// Size of the transaction in bytes (for capacity management).
    pub(crate) size_bytes: usize,
}

impl<P: MempoolPriorityPolicy> MempoolEntry<P> {
    /// Create a new mempool entry.
    pub(crate) fn new(tx: OLTransaction, priority: P::Priority, size_bytes: usize) -> Self {
        Self {
            tx,
            priority,
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
}
