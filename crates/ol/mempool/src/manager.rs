//! Mempool manager with synchronous API.
//!
//! Wraps `MempoolCore` with Arc<Mutex<>> to provide thread-safe synchronous API
//! for RPC and block assembly.

use std::sync::{Arc, Mutex};

use strata_codec::decode_buf_exact;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

use crate::{
    core::MempoolCore,
    error::{MempoolError, MempoolResult},
    types::{MempoolConfig, MempoolStats},
};

/// Mempool manager providing synchronous API.
///
/// This wraps `MempoolCore` with Arc<Mutex<>> and provides methods for:
/// - RPC handlers to submit transactions
/// - Block assembly to retrieve transactions
/// - Management operations (update slot, get stats)
#[derive(Clone)]
pub struct MempoolManager {
    /// Core mempool protected by mutex.
    core: Arc<Mutex<MempoolCore>>,
}

impl MempoolManager {
    /// Create a new mempool manager with the given configuration and current slot.
    pub fn new(config: MempoolConfig, current_slot: u64) -> Self {
        Self {
            core: Arc::new(Mutex::new(MempoolCore::new(config, current_slot))),
        }
    }

    /// Update the current slot (called by FCM-driven loop).
    pub fn update_current_slot(&self, slot: u64) {
        let mut core = self.core.lock().unwrap();
        core.update_current_slot(slot);
    }

    /// Submits a raw transaction to the mempool.
    ///
    /// Accepts a raw transaction blob (opaque bytes) which is parsed into an `OLTransaction`,
    /// validated, and stored. Returns the transaction ID if successful.
    ///
    /// # Errors
    ///
    /// - [`MempoolError::ParseError`] - if the transaction blob cannot be parsed
    /// - [`MempoolError::DuplicateTransaction`] - if the transaction already exists
    /// - [`MempoolError::InvalidTransaction`] - if validation fails
    /// - [`MempoolError::TransactionTooLarge`] - if the transaction exceeds size limits
    /// - [`MempoolError::MempoolCountLimitExceeded`] - if mempool is full
    /// - [`MempoolError::MempoolSizeLimitExceeded`] - if mempool size limit exceeded
    pub fn submit_transaction(&self, blob: Vec<u8>) -> MempoolResult<OLTxId> {
        // Parse the transaction blob
        let tx: OLTransaction = decode_buf_exact(&blob)
            .map_err(|e| MempoolError::ParseError(format!("Failed to parse transaction: {e}")))?;

        let blob_size = blob.len();

        let mut core = self.core.lock().unwrap();
        core.submit_transaction(tx, blob_size)
    }

    /// Retrieves transactions from the mempool for block assembly.
    ///
    /// Returns up to `limit` transactions, ordered by the mempool's ordering policy
    /// (FIFO by entry_slot, then by OLTxId as tie-breaker).
    ///
    /// Returns an empty vector if no transactions are available (not an error).
    pub fn get_transactions(&self, limit: u64) -> MempoolResult<Vec<(OLTxId, OLTransaction)>> {
        let core = self.core.lock().unwrap();
        Ok(core.get_transactions(limit))
    }

    /// Removes transactions from the mempool.
    ///
    /// Typically called after transactions have been included in a block.
    /// Returns the list of transaction IDs that were successfully removed.
    /// Already-removed transactions are silently ignored (idempotent operation).
    pub fn remove_transactions(&self, txids: &[OLTxId]) -> MempoolResult<Vec<OLTxId>> {
        let mut core = self.core.lock().unwrap();
        Ok(core.remove_transactions(txids))
    }

    /// Gets statistics about the current mempool state.
    ///
    /// Returns statistics including transaction count, total size, and rejection counts.
    pub fn stats(&self) -> MempoolResult<MempoolStats> {
        let core = self.core.lock().unwrap();
        Ok(core.stats())
    }

    /// Check if a transaction exists in the mempool.
    pub fn contains(&self, txid: &OLTxId) -> bool {
        let core = self.core.lock().unwrap();
        core.contains(txid)
    }
}

impl std::fmt::Debug for MempoolManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let core = self.core.lock().unwrap();
        f.debug_struct("MempoolManager")
            .field("core", &*core)
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

    fn create_test_tx_blob(payload_bytes: Vec<u8>) -> Vec<u8> {
        let payload = GamTxPayload::new(
            AccountId::new([0u8; 32]),
            VarVec::from_vec(payload_bytes).unwrap(),
        );
        let tx = OLTransaction::new(
            TransactionPayload::GenericAccountMessage(payload),
            TransactionAttachment::new(None, None),
        );
        encode_to_vec(&tx).unwrap()
    }

    #[test]
    fn test_submit_and_get() {
        let config = MempoolConfig::default();
        let manager = MempoolManager::new(config, 100);

        let blob1 = create_test_tx_blob(vec![1, 2, 3]);
        let blob2 = create_test_tx_blob(vec![4, 5, 6]);

        // Submit transactions
        let txid1 = manager.submit_transaction(blob1).unwrap();
        let txid2 = manager.submit_transaction(blob2).unwrap();

        // Get transactions (should be in FIFO order - earliest first)
        let txs = manager.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].0, txid1);
        assert_eq!(txs[1].0, txid2);

        // Check stats
        let stats = manager.stats().unwrap();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.enqueued_tx_total, 2);
    }

    #[test]
    fn test_duplicate_rejection() {
        let config = MempoolConfig::default();
        let manager = MempoolManager::new(config, 100);

        let blob = create_test_tx_blob(vec![1, 2, 3]);

        // First submission succeeds
        manager.submit_transaction(blob.clone()).unwrap();

        // Second submission fails
        let result = manager.submit_transaction(blob);
        assert!(matches!(
            result,
            Err(MempoolError::DuplicateTransaction { .. })
        ));
    }

    #[test]
    fn test_remove_transactions() {
        let config = MempoolConfig::default();
        let manager = MempoolManager::new(config, 100);

        let blob1 = create_test_tx_blob(vec![1, 2, 3]);
        let blob2 = create_test_tx_blob(vec![4, 5, 6]);

        // Submit transactions
        let txid1 = manager.submit_transaction(blob1).unwrap();
        manager.submit_transaction(blob2).unwrap();

        // Remove tx1
        let removed = manager.remove_transactions(&[txid1]).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], txid1);

        // Only tx2 remains
        let txs = manager.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 1);

        // Check stats
        let stats = manager.stats().unwrap();
        assert_eq!(stats.current_tx_count, 1);
    }

    #[test]
    fn test_capacity_limits() {
        let config = MempoolConfig {
            max_tx_count: 2,
            max_tx_size: 500,
            max_txs_per_account: 16,
        };
        let manager = MempoolManager::new(config, 100);

        let blob1 = create_test_tx_blob(vec![1; 200]);
        let blob2 = create_test_tx_blob(vec![2; 200]);
        let blob3 = create_test_tx_blob(vec![3; 200]);

        // Submit first two transactions
        manager.submit_transaction(blob1).unwrap();
        manager.submit_transaction(blob2).unwrap();

        // Third submission should evict tx1 (lowest priority = earliest)
        manager.submit_transaction(blob3).unwrap();

        // Check that tx3 was added and tx1 was evicted
        let txs = manager.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 2);

        // Check stats
        let stats = manager.stats().unwrap();
        assert_eq!(stats.current_tx_count, 2);
        assert_eq!(stats.evicted_tx_total, 1);
    }

    #[test]
    fn test_fifo_ordering() {
        let config = MempoolConfig::default();
        let manager = MempoolManager::new(config, 100);

        // Submit transactions with increasing entry slots
        for i in 0..5 {
            manager.update_current_slot(100 + i);
            let blob = create_test_tx_blob(vec![i as u8]);
            manager.submit_transaction(blob).unwrap();
        }

        // Get transactions - should be in FIFO order (earliest first)
        let txs = manager.get_transactions(10).unwrap();
        assert_eq!(txs.len(), 5);
    }

    #[test]
    fn test_get_transactions_limit() {
        let config = MempoolConfig::default();
        let manager = MempoolManager::new(config, 100);

        // Submit 5 transactions
        for i in 0..5 {
            manager.update_current_slot(100 + i);
            let blob = create_test_tx_blob(vec![i as u8]);
            manager.submit_transaction(blob).unwrap();
        }

        // Request only 3 transactions
        let txs = manager.get_transactions(3).unwrap();
        assert_eq!(txs.len(), 3);
    }

    #[test]
    fn test_contains() {
        let config = MempoolConfig::default();
        let manager = MempoolManager::new(config, 100);

        let blob = create_test_tx_blob(vec![1, 2, 3]);
        let txid = manager.submit_transaction(blob).unwrap();

        assert!(manager.contains(&txid));
        assert!(!manager.contains(&OLTxId::from(Buf32([0u8; 32]))));
    }
}
