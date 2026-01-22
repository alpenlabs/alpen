//! Mempool service handle for external interaction.

use strata_identifiers::OLTxId;
use strata_service::ServiceMonitor;
use tokio::sync::{mpsc, oneshot};

use crate::{
    MempoolCommand, OLMempoolError, OLMempoolResult,
    command::create_completion,
    service::MempoolServiceStatus,
    types::{MempoolTxRemovalReason, OLMempoolStats, OLMempoolTransaction},
};

/// Handle for interacting with the mempool service.
#[derive(Debug, Clone)]
pub struct MempoolHandle {
    command_tx: mpsc::Sender<MempoolCommand>,
    monitor: ServiceMonitor<MempoolServiceStatus>,
}

impl MempoolHandle {
    /// Create a new mempool handle (used by builder).
    pub(crate) fn new(
        command_tx: mpsc::Sender<MempoolCommand>,
        monitor: ServiceMonitor<MempoolServiceStatus>,
    ) -> Self {
        Self {
            command_tx,
            monitor,
        }
    }

    /// Helper to map send/recv errors to ServiceClosed.
    fn service_closed_error<T>(_: T) -> OLMempoolError {
        OLMempoolError::ServiceClosed("Mempool service closed".to_string())
    }

    /// Send command and wait for response.
    async fn send_command<R>(
        &self,
        command: MempoolCommand,
        rx: oneshot::Receiver<R>,
    ) -> OLMempoolResult<R> {
        self.command_tx
            .send(command)
            .await
            .map_err(Self::service_closed_error)?;

        rx.await.map_err(Self::service_closed_error)
    }

    /// Submit a transaction to the mempool.
    ///
    /// # Arguments
    /// * `tx` - The transaction to submit
    ///
    /// # Returns
    /// The transaction ID if successfully added
    pub async fn submit_transaction(&self, tx: OLMempoolTransaction) -> OLMempoolResult<OLTxId> {
        let (completion, rx) = create_completion();
        let command = MempoolCommand::SubmitTransaction {
            tx: Box::new(tx),
            completion,
        };
        self.send_command(command, rx).await?
    }

    /// Get transactions from the mempool in priority order.
    ///
    /// Returns up to `limit` transactions in priority order.
    /// Use `usize::MAX` to get all transactions.
    pub async fn get_transactions(
        &self,
        limit: usize,
    ) -> OLMempoolResult<Vec<(OLTxId, OLMempoolTransaction)>> {
        let (completion, rx) = create_completion();
        let command = MempoolCommand::GetTransactions { completion, limit };
        self.send_command(command, rx).await?
    }

    /// Remove transactions from the mempool.
    ///
    /// # Arguments
    /// * `txs` - Vector of (transaction_id, removal_reason) pairs
    ///
    /// # Returns
    /// Vector of transaction IDs that were successfully removed
    pub async fn remove_transactions(
        &self,
        txs: Vec<(OLTxId, MempoolTxRemovalReason)>,
    ) -> OLMempoolResult<Vec<OLTxId>> {
        let (completion, rx) = create_completion();
        let command = MempoolCommand::RemoveTransactions { txs, completion };
        self.send_command(command, rx).await?
    }

    /// Get mempool statistics.
    pub fn stats(&self) -> OLMempoolStats {
        self.monitor.get_current().stats
    }

    /// Get a reference to the service monitor for status updates.
    pub fn monitor(&self) -> &ServiceMonitor<MempoolServiceStatus> {
        &self.monitor
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_csm_types::{ClientState, L1Status};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{L1BlockCommitment, L1BlockId};
    use strata_status::StatusChannel;
    use strata_storage::{NodeStorage, create_node_storage};
    use strata_tasks::TaskManager;
    use threadpool::ThreadPool;
    use tokio::runtime::Handle;

    use super::*;
    use crate::{
        MempoolBuilder,
        test_utils::{
            create_test_block_commitment, create_test_generic_tx_for_account,
            create_test_snark_tx_with_seq_no, create_test_snark_tx_with_seq_no_and_slots,
            setup_test_state_for_tip,
        },
        types::{MempoolTxRemovalReason, OLMempoolConfig},
    };

    /// Helper to set up mempool handle with storage for tests.
    /// Returns (handle, storage, status_channel) for triggering chain updates.
    async fn setup_mempool() -> (MempoolHandle, Arc<NodeStorage>, StatusChannel) {
        let pool = ThreadPool::new(1);
        let test_db = get_test_sled_backend();
        let storage = Arc::new(
            create_node_storage(test_db, pool).expect("Failed to create test NodeStorage"),
        );

        let config = OLMempoolConfig::default();
        let current_tip = create_test_block_commitment(100);

        // Set up test state for the tip and other tips tests will use
        setup_test_state_for_tip(&storage, current_tip).await;
        setup_test_state_for_tip(&storage, create_test_block_commitment(80)).await;
        setup_test_state_for_tip(&storage, create_test_block_commitment(160)).await;
        setup_test_state_for_tip(&storage, create_test_block_commitment(200)).await;

        let client_state = ClientState::new(None, None);
        let l1_block = L1BlockCommitment::from_height_u64(0, L1BlockId::default())
            .expect("Failed to create L1BlockCommitment");
        let l1_status = L1Status::default();
        let status_channel = StatusChannel::new(client_state, l1_block, l1_status, None, None);

        let task_manager = TaskManager::new(Handle::current());
        let texec = task_manager.create_executor();

        let handle =
            MempoolBuilder::new(config, storage.clone(), status_channel.clone(), current_tip)
                .launch(&texec)
                .await
                .unwrap();

        (handle, storage, status_channel)
    }

    /// Helper to check if transaction exists in database (test-only).
    fn tx_exists(storage: &NodeStorage, txid: OLTxId) -> bool {
        storage.mempool().get_tx(txid).unwrap().is_some()
    }

    #[test]
    fn test_service_closed_error() {
        let err = MempoolHandle::service_closed_error(());
        assert!(matches!(err, OLMempoolError::ServiceClosed(_)));
    }

    #[tokio::test]
    async fn test_launch_with_status_channel() {
        let (handle, _storage, _status_channel) = setup_mempool().await;
        let stats = handle.stats();
        assert_eq!(stats.mempool_size(), 0);
    }

    #[tokio::test]
    async fn test_submit_and_contains() {
        let (handle, storage, _status_channel) = setup_mempool().await;

        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let txid = handle.submit_transaction(tx).await.unwrap();

        assert!(tx_exists(&storage, txid));
        assert_eq!(handle.stats().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_get_transactions_and_remove() {
        let (handle, storage, _status_channel) = setup_mempool().await;

        // Use different accounts to ensure unique transaction IDs
        let tx1 = create_test_generic_tx_for_account(1);
        let tx2 = create_test_generic_tx_for_account(2);
        let tx3 = create_test_generic_tx_for_account(3);

        let txid1 = handle.submit_transaction(tx1).await.unwrap();
        let txid2 = handle.submit_transaction(tx2).await.unwrap();
        let txid3 = handle.submit_transaction(tx3).await.unwrap();

        // Get transactions
        let txs = handle.get_transactions(10).await.unwrap();
        assert_eq!(txs.len(), 3);

        // Remove one transaction
        let removed = handle
            .remove_transactions(vec![(txid2, MempoolTxRemovalReason::Included)])
            .await
            .unwrap();
        assert_eq!(removed.len(), 1);

        // Verify tx2 removed, others remain
        assert!(!tx_exists(&storage, txid2));
        assert!(tx_exists(&storage, txid1));
        assert!(tx_exists(&storage, txid3));
        assert_eq!(handle.stats().mempool_size(), 2);
    }

    #[tokio::test]
    async fn test_get_transactions_with_limit() {
        let (handle, storage, _status_channel) = setup_mempool().await;

        // Use different accounts to ensure unique transaction IDs
        let tx1 = create_test_generic_tx_for_account(1);
        let tx2 = create_test_generic_tx_for_account(2);
        let tx3 = create_test_generic_tx_for_account(3);

        let txid1 = handle.submit_transaction(tx1).await.unwrap();
        let txid2 = handle.submit_transaction(tx2).await.unwrap();
        let txid3 = handle.submit_transaction(tx3).await.unwrap();

        // Get transactions with limit
        let txs = handle.get_transactions(2).await.unwrap();
        assert_eq!(txs.len(), 2);

        // Verify all transactions still exist in storage
        assert!(tx_exists(&storage, txid1));
        assert!(tx_exists(&storage, txid2));
        assert!(tx_exists(&storage, txid3));
        assert_eq!(handle.stats().mempool_size(), 3);
    }

    #[tokio::test]
    async fn test_duplicate_transaction_idempotent() {
        let (handle, _storage, _status_channel) = setup_mempool().await;

        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let tx_clone = tx.clone();

        let txid1 = handle.submit_transaction(tx).await.unwrap();
        let txid2 = handle.submit_transaction(tx_clone).await.unwrap();

        assert_eq!(txid1, txid2, "Same tx should have same txid");
        assert_eq!(handle.stats().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_transaction_min_slot_validation() {
        let (handle, _storage, _status_channel) = setup_mempool().await;

        // Add tx valid from slot 200 (current tip is 100)
        let tx_future = create_test_snark_tx_with_seq_no_and_slots(1, 0, Some(200), None);

        // Should be rejected at validation time
        let result = handle.submit_transaction(tx_future).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::TransactionNotMature { .. }
        ));
    }
}
