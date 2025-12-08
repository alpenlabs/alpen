//! Core mempool types.

use std::{cmp::Ordering, collections::BTreeMap};

use ssz_derive::{Decode, Encode};
use strata_acct_types::AccountId;
use strata_identifiers::{OLTxId, hash};
use strata_ol_chain_types_new::{GamTxPayload, TransactionAttachment};
use strata_snark_acct_types::SnarkAccountUpdate;

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
}

impl Default for OLMempoolConfig {
    fn default() -> Self {
        Self {
            max_tx_count: DEFAULT_MAX_TX_COUNT,
            max_tx_size: DEFAULT_MAX_TX_SIZE,
            max_mempool_bytes: DEFAULT_MAX_MEMPOOL_BYTES,
            max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
        }
    }
}

/// Snark account update payload for mempool (without accumulator proofs).
///
/// This is similar to
/// [`SnarkAccountUpdateTxPayload`](strata_ol_chain_types_new::SnarkAccountUpdateTxPayload)
/// but stores only the [`SnarkAccountUpdate`](strata_snark_acct_types::SnarkAccountUpdate) without
/// the `accumulator_proofs` that are
/// part of [`SnarkAccountUpdateContainer`](strata_snark_acct_types::SnarkAccountUpdateContainer).
/// During block assembly, accumulator proofs are generated and this is converted to
/// a full [`SnarkAccountUpdateTxPayload`](strata_ol_chain_types_new::SnarkAccountUpdateTxPayload)
/// with a complete
/// [`SnarkAccountUpdateContainer`](strata_snark_acct_types::SnarkAccountUpdateContainer).
#[derive(Clone, Debug, Encode, Decode, PartialEq, Eq)]
pub struct OLMempoolSnarkAcctUpdateTxPayload {
    /// Target account for this transaction.
    pub target: AccountId,
    /// Base snark account update WITHOUT accumulator proofs.
    pub base_update: SnarkAccountUpdate,
}

impl OLMempoolSnarkAcctUpdateTxPayload {
    /// Create a new snark account update mempool payload.
    pub fn new(target: AccountId, base_update: SnarkAccountUpdate) -> Self {
        Self {
            target,
            base_update,
        }
    }

    /// Get the target account.
    pub fn target(&self) -> &AccountId {
        &self.target
    }

    /// Get the base update.
    pub fn base_update(&self) -> &SnarkAccountUpdate {
        &self.base_update
    }
}

/// Transaction payload for mempool.
///
/// This represents the payload portion of a mempool transaction WITHOUT accumulator proofs.
/// It mirrors [`TransactionPayload`](strata_ol_chain_types_new::TransactionPayload) but for
/// SnarkAccountUpdate, uses a minimal structure without accumulator_proofs. During block assembly,
/// accumulator proofs are generated and this is converted to a full
/// [`OLTransaction`](strata_ol_chain_types_new::OLTransaction).
#[derive(Clone, Debug, Encode, Decode, PartialEq, Eq)]
#[ssz(enum_behaviour = "union")]
pub enum OLMempoolTxPayload {
    /// Generic account message transaction.
    GenericAccountMessage(GamTxPayload),

    /// Snark account update transaction WITHOUT accumulator proofs.
    SnarkAccountUpdate(OLMempoolSnarkAcctUpdateTxPayload),
}

impl OLMempoolTxPayload {
    /// Get the target account for this transaction payload.
    pub fn target(&self) -> AccountId {
        match self {
            OLMempoolTxPayload::GenericAccountMessage(gam) => *gam.target(),
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => *payload.target(),
        }
    }

    /// Create a new generic account message payload.
    pub fn new_generic_account_message(
        target: AccountId,
        payload: Vec<u8>,
    ) -> Result<Self, &'static str> {
        Ok(Self::GenericAccountMessage(GamTxPayload::new(
            target, payload,
        )?))
    }

    /// Create a new snark account update payload without accumulator proofs.
    pub fn new_snark_account_update(target: AccountId, base_update: SnarkAccountUpdate) -> Self {
        Self::SnarkAccountUpdate(OLMempoolSnarkAcctUpdateTxPayload::new(target, base_update))
    }
}

/// Transaction data for mempool.
///
/// This contains the payload and attachment needed for an OL transaction WITHOUT accumulator
/// proofs. During block assembly, accumulator proofs are generated and this is converted to a full
/// [`OLTransaction`](strata_ol_chain_types_new::OLTransaction).
///
/// The transaction ID ([`OLTxId`]) is computed by SSZ-encoding this structure and hashing it.
/// This hash equals the hash of [`OLTransaction`](strata_ol_chain_types_new::OLTransaction)
/// without accumulator proofs, ensuring consistency across mempool, canonical, and RPC
/// representations.
#[derive(Clone, Debug, Encode, Decode, PartialEq, Eq)]
pub struct OLMempoolTransaction {
    /// Transaction payload (generic message or snark account update).
    pub payload: OLMempoolTxPayload,

    /// Transaction attachment (min_slot, max_slot constraints).
    pub attachment: TransactionAttachment,
}

impl OLMempoolTransaction {
    /// Create a new mempool transaction with a generic account message.
    pub fn new_generic_account_message(
        target: AccountId,
        payload: Vec<u8>,
        attachment: TransactionAttachment,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            payload: OLMempoolTxPayload::new_generic_account_message(target, payload)?,
            attachment,
        })
    }

    /// Create a new mempool transaction with a snark account update.
    pub fn new_snark_account_update(
        target: AccountId,
        base_update: SnarkAccountUpdate,
        attachment: TransactionAttachment,
    ) -> Self {
        Self {
            payload: OLMempoolTxPayload::new_snark_account_update(target, base_update),
            attachment,
        }
    }

    /// Get the target account.
    pub fn target(&self) -> AccountId {
        self.payload.target()
    }

    /// Get the payload.
    pub fn payload(&self) -> &OLMempoolTxPayload {
        &self.payload
    }

    /// Get the attachment.
    pub fn attachment(&self) -> &TransactionAttachment {
        &self.attachment
    }

    /// Get the base update if this is a snark account update transaction.
    pub fn base_update(&self) -> Option<&SnarkAccountUpdate> {
        match &self.payload {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => Some(payload.base_update()),
            OLMempoolTxPayload::GenericAccountMessage(_) => None,
        }
    }

    /// Compute the transaction ID by hashing the SSZ-encoded transaction data.
    ///
    /// This follows the established pattern: SSZ encode → SHA256 hash.
    /// The hash of [`OLMempoolTransaction`] should equal the hash of
    /// [`OLTransaction`](strata_ol_chain_types_new::OLTransaction) without accumulator_proofs,
    /// and also equal the hash of `RpcOLTransaction`
    /// when properly converted.
    pub fn compute_txid(&self) -> OLTxId {
        let encoded = ssz::Encode::as_ssz_bytes(self);
        let hash_bytes = hash::raw(&encoded);
        OLTxId::from(hash_bytes)
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
    pub(crate) fn for_transaction(tx: &OLMempoolTransaction, timestamp_micros: u64) -> Self {
        match tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => Self::Snark {
                account_id: *payload.target(),
                seq_no: payload.base_update().operation().seq_no(),
                timestamp_micros,
            },
            OLMempoolTxPayload::GenericAccountMessage(_) => Self::Gam { timestamp_micros },
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
    pub(crate) tx: OLMempoolTransaction,

    /// Ordering key.
    pub(crate) ordering_key: MempoolOrderingKey,

    /// Size of the transaction in bytes (for capacity management).
    pub(crate) size_bytes: usize,
}

impl MempoolEntry {
    /// Create a new mempool entry.
    pub(crate) fn new(
        tx: OLMempoolTransaction,
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
            | OLMempoolError::Serialization(_)
            | OLMempoolError::ServiceClosed(_) => None,
        }
    }
}

/// Reason for removing a transaction from the mempool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MempoolTxRemovalReason {
    /// Transaction was included in a block.
    Included,

    /// Transaction was replaced by a newer version with the same sequence number.
    Replaced,

    /// Transaction validation failed (expired, invalid seqno, etc.).
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
    use proptest::{prelude::*, strategy::ValueTree, test_runner::TestRunner};
    use ssz::Decode;
    use strata_acct_types::AccountId;
    use strata_identifiers::Buf32;
    use strata_ol_chain_types_new::test_utils;
    use strata_snark_acct_types::{
        LedgerRefs, ProofState, SnarkAccountUpdate, UpdateOperationData, UpdateOutputs,
    };

    use super::*;
    use crate::test_utils::{
        create_test_account_id, create_test_attachment, create_test_snark_update,
    };

    /// Proptest strategy for creating mempool snark account update payloads.
    ///
    /// Uses strategies from ol/chain-types to generate meaningful test data.
    fn mempool_snark_update_tx_payload_strategy()
    -> impl Strategy<Value = OLMempoolSnarkAcctUpdateTxPayload> {
        (
            any::<[u8; 32]>(),                                                     // target
            any::<[u8; 32]>(), // proof_state inner_state
            any::<u64>(),      // seq_no
            prop::collection::vec(test_utils::message_entry_strategy(), 0..5), // messages
            prop::collection::vec(test_utils::accumulator_claim_strategy(), 0..3), // ledger_refs
            prop::collection::vec(test_utils::output_transfer_strategy(), 0..3), // output_transfers
            prop::collection::vec(test_utils::output_message_strategy(), 0..3), // output_messages
            prop::collection::vec(any::<u8>(), 0..32), // extra_data
            prop::collection::vec(any::<u8>(), 0..100), // update_proof
        )
            .prop_map(
                |(
                    target_bytes,
                    state_bytes,
                    seq_no,
                    messages,
                    ledger_refs,
                    output_transfers,
                    output_messages,
                    extra_data,
                    update_proof,
                )| {
                    let proof_state = ProofState::new(Buf32::from(state_bytes), 0);
                    let operation = UpdateOperationData::new(
                        seq_no,
                        proof_state,
                        messages,
                        LedgerRefs::new(ledger_refs),
                        UpdateOutputs::new(output_transfers, output_messages),
                        extra_data,
                    );
                    let base_update = SnarkAccountUpdate::new(operation, update_proof);
                    OLMempoolSnarkAcctUpdateTxPayload::new(
                        AccountId::from(target_bytes),
                        base_update,
                    )
                },
            )
    }

    /// Proptest strategy for creating mempool transaction payloads.
    ///
    /// Reuses strategies from ol/chain-types for GenericAccountMessage and creates
    /// a custom strategy for SnarkAccountUpdate (without accumulator proofs).
    fn mempool_tx_payload_strategy() -> impl Strategy<Value = OLMempoolTxPayload> {
        prop_oneof![
            test_utils::gam_tx_payload_strategy()
                .prop_map(OLMempoolTxPayload::GenericAccountMessage),
            mempool_snark_update_tx_payload_strategy()
                .prop_map(OLMempoolTxPayload::SnarkAccountUpdate),
        ]
    }

    /// Proptest strategy for creating mempool transactions.
    ///
    /// Reuses transaction_attachment_strategy from ol/chain-types.
    fn mempool_transaction_strategy() -> impl Strategy<Value = OLMempoolTransaction> {
        (
            mempool_tx_payload_strategy(),
            test_utils::transaction_attachment_strategy(),
        )
            .prop_map(|(payload, attachment)| OLMempoolTransaction {
                payload,
                attachment,
            })
    }

    #[test]
    fn test_generic_account_message_creation() {
        let mut runner = TestRunner::default();
        let payload_strategy = prop::collection::vec(any::<u8>(), 10..100);
        let payload = payload_strategy.new_tree(&mut runner).unwrap().current();

        let target = create_test_account_id();
        let attachment = create_test_attachment();

        let tx = OLMempoolTransaction::new_generic_account_message(
            target,
            payload.clone(),
            attachment.clone(),
        )
        .expect("Should create generic account message tx");

        assert_eq!(tx.target(), target);
        assert_eq!(tx.attachment(), &attachment);
        assert!(tx.base_update().is_none());

        match tx.payload() {
            OLMempoolTxPayload::GenericAccountMessage(gam) => {
                assert_eq!(gam.target(), &target);
                assert_eq!(gam.payload(), payload.as_slice());
            }
            _ => panic!("Expected GenericAccountMessage"),
        }
    }

    #[test]
    fn test_snark_account_update_creation() {
        let target = create_test_account_id();
        let base_update = create_test_snark_update();
        let attachment = create_test_attachment();

        let tx = OLMempoolTransaction::new_snark_account_update(
            target,
            base_update.clone(),
            attachment.clone(),
        );

        assert_eq!(tx.target(), target);
        assert_eq!(tx.attachment(), &attachment);
        assert_eq!(tx.base_update(), Some(&base_update));

        match tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => {
                assert_eq!(payload.target(), &target);
                assert_eq!(payload.base_update(), &base_update);
            }
            _ => panic!("Expected SnarkAccountUpdate"),
        }
    }

    #[test]
    fn test_compute_txid_generic_message() {
        let mut runner = TestRunner::default();

        // Generate random payloads using proptest
        let payload_strategy = prop::collection::vec(any::<u8>(), 10..100);
        let payload = payload_strategy.new_tree(&mut runner).unwrap().current();

        let target = create_test_account_id();
        let attachment = create_test_attachment();

        let tx1 = OLMempoolTransaction::new_generic_account_message(
            target,
            payload.clone(),
            attachment.clone(),
        )
        .expect("Should create tx");
        let tx2 = OLMempoolTransaction::new_generic_account_message(target, payload, attachment)
            .expect("Should create tx");

        // Same transaction should have same ID
        assert_eq!(tx1.compute_txid(), tx2.compute_txid());

        // Different payload should have different ID
        let different_payload = payload_strategy.new_tree(&mut runner).unwrap().current();
        let tx3 = OLMempoolTransaction::new_generic_account_message(
            target,
            different_payload,
            create_test_attachment(),
        )
        .expect("Should create tx");
        assert_ne!(tx1.compute_txid(), tx3.compute_txid());
    }

    #[test]
    fn test_compute_txid_snark_update() {
        let target = create_test_account_id();
        let base_update = create_test_snark_update();
        let attachment = create_test_attachment();

        let tx1 = OLMempoolTransaction::new_snark_account_update(
            target,
            base_update.clone(),
            attachment.clone(),
        );
        let tx2 = OLMempoolTransaction::new_snark_account_update(target, base_update, attachment);

        // Same transaction should have same ID
        assert_eq!(tx1.compute_txid(), tx2.compute_txid());

        // Different base_update should have different ID
        let different_base_update = create_test_snark_update();
        let tx3 = OLMempoolTransaction::new_snark_account_update(
            target,
            different_base_update,
            create_test_attachment(),
        );
        assert_ne!(tx1.compute_txid(), tx3.compute_txid());
    }

    #[test]
    fn test_ssz_roundtrip_generic_message() {
        let mut runner = TestRunner::default();
        let payload_strategy = prop::collection::vec(any::<u8>(), 10..100);
        let payload = payload_strategy.new_tree(&mut runner).unwrap().current();

        let target = create_test_account_id();
        let attachment = create_test_attachment();

        let tx = OLMempoolTransaction::new_generic_account_message(target, payload, attachment)
            .expect("Should create tx");

        let encoded = ssz::Encode::as_ssz_bytes(&tx);
        let decoded = OLMempoolTransaction::from_ssz_bytes(&encoded).expect("Should decode");

        assert_eq!(tx, decoded);
    }

    #[test]
    fn test_ssz_roundtrip_snark_update() {
        let target = create_test_account_id();
        let base_update = create_test_snark_update();
        let attachment = create_test_attachment();

        let tx = OLMempoolTransaction::new_snark_account_update(target, base_update, attachment);

        let encoded = ssz::Encode::as_ssz_bytes(&tx);
        let decoded = OLMempoolTransaction::from_ssz_bytes(&encoded).expect("Should decode");

        assert_eq!(tx, decoded);
    }

    proptest::proptest! {
        #[test]
        fn test_mempool_tx_ssz_roundtrip(tx in mempool_transaction_strategy()) {
            let encoded = ssz::Encode::as_ssz_bytes(&tx);
            let decoded = OLMempoolTransaction::from_ssz_bytes(&encoded)
                .expect("Should decode mempool transaction");
            prop_assert_eq!(tx, decoded);
        }

        #[test]
        fn test_mempool_tx_id_consistency(tx in mempool_transaction_strategy()) {
            let txid1 = tx.compute_txid();
            let txid2 = tx.compute_txid();
            prop_assert_eq!(txid1, txid2, "Transaction ID should be deterministic");
        }

        #[test]
        fn test_mempool_tx_payload_target(
            payload in mempool_tx_payload_strategy()
        ) {
            let target = payload.target();
            prop_assert!(target != AccountId::zero(), "Target should not be zero");
        }
    }

    // Tests for MempoolOrderingKey::Ord implementation
    mod ordering_tests {
        use std::cmp::Ordering;

        use strata_acct_types::AccountId;

        use super::*;
        use crate::test_utils::{create_test_generic_tx, create_test_snark_tx};

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
            let account = AccountId::from([1u8; 32]);
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
            let account_a = AccountId::from([1u8; 32]);
            let account_b = AccountId::from([2u8; 32]);

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
            let account = AccountId::from([1u8; 32]);

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
            let (acc_a, acc_b) = (AccountId::from([1u8; 32]), AccountId::from([2u8; 32]));
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
