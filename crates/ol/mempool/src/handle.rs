//! Mempool service handle for external interaction.

use std::sync::Arc;

use strata_identifiers::{OLBlockCommitment, OLTxId};
use strata_service::{CommandHandle, ServiceBuilder, ServiceMonitor};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tracing::warn;

use crate::{
    MempoolCommand, OLMempoolError, OLMempoolResult,
    command::create_completion,
    service::{MempoolService, MempoolServiceStatus},
    state::{MempoolContext, MempoolServiceState},
    types::{OLMempoolConfig, OLMempoolStats, OLMempoolTransaction},
};

/// Handle for interacting with the mempool service.
#[derive(Debug)]
pub struct MempoolHandle {
    command_handle: Arc<CommandHandle<MempoolCommand>>,
    monitor: ServiceMonitor<MempoolServiceStatus>,
}

impl MempoolHandle {
    /// Helper to map send/recv errors to ServiceClosed.
    fn service_closed_error<T>(_: T) -> OLMempoolError {
        OLMempoolError::ServiceClosed("Mempool service closed".to_string())
    }

    /// Send command and wait for response.
    async fn send_command<R>(
        &self,
        command: MempoolCommand,
        rx: tokio::sync::oneshot::Receiver<R>,
    ) -> OLMempoolResult<R> {
        self.command_handle
            .send(command)
            .await
            .map_err(Self::service_closed_error)?;

        rx.await.map_err(Self::service_closed_error)
    }

    /// Create and launch a new mempool service.
    ///
    /// # Arguments
    /// * `storage` - Node storage (provides both database manager and StateAccessor creation)
    /// * `config` - Mempool configuration
    /// * `current_tip` - Current chain tip (slot + block ID)
    /// * `status_channel` - Status channel for subscribing to chain sync updates
    /// * `texec` - Task executor for spawning the service task
    ///
    /// # Returns
    /// A handle to interact with the launched service
    pub async fn launch(
        storage: Arc<NodeStorage>,
        config: OLMempoolConfig,
        current_tip: OLBlockCommitment,
        status_channel: StatusChannel,
        texec: &TaskExecutor,
    ) -> anyhow::Result<Self> {
        let ctx = Arc::new(MempoolContext::new(config, storage));
        let mut state = MempoolServiceState::new_with_context(ctx.clone(), current_tip);

        // Load existing transactions from database
        state.load_from_db()?;

        // Create service builder
        let mut service_builder = ServiceBuilder::<MempoolService, _>::new().with_state(state);

        // Create command handle with configured buffer size
        let command_handle =
            Arc::new(service_builder.create_command_handle(ctx.config.command_buffer_size));

        // Spawn background worker to subscribe to chain sync updates
        let command_handle_clone = Arc::clone(&command_handle);
        let mut chain_sync_rx = status_channel.subscribe_chain_sync();
        texec.spawn_critical_async("mempool_chain_sync", async move {
            loop {
                if chain_sync_rx.changed().await.is_err() {
                    // Channel closed, exit
                    break;
                }

                // Clone the update before await to avoid Send issues
                let new_tip = chain_sync_rx
                    .borrow_and_update()
                    .as_ref()
                    .map(|update| update.new_status().tip);

                if let Some(tip) = new_tip {
                    let (completion, _rx) = create_completion();
                    let cmd = MempoolCommand::ChainUpdate {
                        new_tip: tip,
                        completion,
                    };
                    if let Err(e) = command_handle_clone.send(cmd).await {
                        warn!(?e, "Failed to send chain update to mempool service");
                    }
                }
            }
            Ok(())
        });

        // Launch service
        let monitor = service_builder.launch_async("mempool", texec).await?;

        Ok(Self {
            command_handle,
            monitor,
        })
    }

    /// Submit a transaction to the mempool.
    ///
    /// # Arguments
    /// * `tx` - The transaction to submit
    ///
    /// # Returns
    /// The transaction ID if successfully added
    pub async fn submit_transaction(&self, tx: OLMempoolTransaction) -> OLMempoolResult<OLTxId> {
        let tx_bytes = ssz::Encode::as_ssz_bytes(&tx);
        let (completion, rx) = create_completion();
        let command = MempoolCommand::SubmitTransaction {
            tx_bytes,
            completion,
        };
        self.send_command(command, rx).await?
    }

    /// Get all best transactions from the mempool in priority order.
    ///
    /// Returns all transactions for use with the iterator pattern.
    /// For limited queries, callers can use `.take(limit)` on the result.
    pub async fn best_transactions(&self) -> OLMempoolResult<Vec<(OLTxId, OLMempoolTransaction)>> {
        let (completion, rx) = create_completion();
        let command = MempoolCommand::BestTransactions { completion };
        self.send_command(command, rx).await?
    }

    /// Remove transactions from the mempool (after block inclusion).
    ///
    /// # Returns
    /// Vector of transaction IDs that were successfully removed
    pub async fn remove_transactions(&self, ids: Vec<OLTxId>) -> OLMempoolResult<Vec<OLTxId>> {
        let (completion, rx) = create_completion();
        let command = MempoolCommand::RemoveTransactions { ids, completion };
        self.send_command(command, rx).await?
    }

    /// Check if a transaction exists in the mempool.
    pub async fn contains(&self, id: OLTxId) -> OLMempoolResult<bool> {
        let (completion, rx) = create_completion();
        let command = MempoolCommand::Contains { id, completion };
        self.send_command(command, rx).await
    }

    /// Get mempool statistics.
    pub async fn stats(&self) -> OLMempoolResult<OLMempoolStats> {
        let (completion, rx) = create_completion();
        let command = MempoolCommand::Stats { completion };
        self.send_command(command, rx).await
    }

    /// Notify mempool of chain tip update (from fork-choice manager).
    ///
    /// Updates the mempool's view of the chain tip and removes expired transactions.
    /// Returns the count of transactions removed.
    pub async fn chain_update(&self, new_tip: OLBlockCommitment) -> OLMempoolResult<usize> {
        let (completion, rx) = create_completion();
        let command = MempoolCommand::ChainUpdate {
            new_tip,
            completion,
        };
        self.send_command(command, rx).await?
    }

    /// Get a reference to the service monitor for status updates.
    pub fn monitor(&self) -> &ServiceMonitor<MempoolServiceStatus> {
        &self.monitor
    }
}

#[cfg(test)]
mod tests {
    use strata_csm_types::{ClientState, L1Status};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{L1BlockCommitment, L1BlockId};
    use strata_status::StatusChannel;
    use strata_storage::{NodeStorage, create_node_storage};
    use strata_tasks::TaskManager;
    use threadpool::ThreadPool;

    use super::*;
    use crate::{
        BestTransactions, BestTransactionsIterator,
        test_utils::{
            create_test_block_commitment, create_test_generic_tx_for_account,
            create_test_snark_tx_with_seq_no, create_test_tx_with_expiry, setup_test_state_for_tip,
        },
        types::OLMempoolConfig,
    };

    /// Helper to set up mempool handle with storage for tests.
    async fn setup_mempool() -> (MempoolHandle, Arc<NodeStorage>) {
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
        let status_channel = StatusChannel::new(client_state, l1_block, l1_status, None);

        let task_manager = TaskManager::new(tokio::runtime::Handle::current());
        let texec = task_manager.create_executor();

        let handle =
            MempoolHandle::launch(storage.clone(), config, current_tip, status_channel, &texec)
                .await
                .unwrap();

        (handle, storage)
    }

    #[test]
    fn test_service_closed_error() {
        let err = MempoolHandle::service_closed_error(());
        assert!(matches!(err, OLMempoolError::ServiceClosed(_)));
    }

    #[tokio::test]
    async fn test_launch_with_status_channel() {
        let (handle, _storage) = setup_mempool().await;
        let stats = handle.stats().await.expect("Should get stats");
        assert_eq!(stats.mempool_size(), 0);
    }

    #[tokio::test]
    async fn test_submit_and_contains() {
        let (handle, _storage) = setup_mempool().await;

        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let txid = handle.submit_transaction(tx).await.unwrap();

        assert!(handle.contains(txid).await.unwrap());
        assert_eq!(handle.stats().await.unwrap().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_best_transactions_and_remove() {
        let (handle, _storage) = setup_mempool().await;

        // Use different accounts to ensure unique transaction IDs
        let tx1 = create_test_generic_tx_for_account(1);
        let tx2 = create_test_generic_tx_for_account(2);
        let tx3 = create_test_generic_tx_for_account(3);

        let txid1 = handle.submit_transaction(tx1).await.unwrap();
        let txid2 = handle.submit_transaction(tx2).await.unwrap();
        let txid3 = handle.submit_transaction(tx3).await.unwrap();

        // Get best transactions
        let txs = handle.best_transactions().await.unwrap();
        assert_eq!(txs.len(), 3);

        // Use iterator pattern to mark one invalid
        let mut iter = BestTransactionsIterator::new(txs);
        while let Some((txid, _tx)) = iter.next() {
            if txid == txid2 {
                iter.mark_invalid(txid);
            }
        }

        // Remove invalid transactions
        let invalid: Vec<_> = iter.into_marked_invalid().into_iter().collect();
        assert_eq!(invalid.len(), 1);

        let removed = handle.remove_transactions(invalid).await.unwrap();
        assert!(!removed.is_empty());

        // Verify tx2 removed, others remain
        assert!(!handle.contains(txid2).await.unwrap());
        assert!(handle.contains(txid1).await.unwrap());
        assert!(handle.contains(txid3).await.unwrap());
        assert_eq!(handle.stats().await.unwrap().mempool_size(), 2);
    }

    #[tokio::test]
    async fn test_best_transactions_trait_interface() {
        let (handle, _storage) = setup_mempool().await;

        // Use different accounts to ensure unique transaction IDs
        let tx1 = create_test_generic_tx_for_account(1);
        let tx2 = create_test_generic_tx_for_account(2);
        let tx3 = create_test_generic_tx_for_account(3);

        let txid1 = handle.submit_transaction(tx1).await.unwrap();
        let txid2 = handle.submit_transaction(tx2).await.unwrap();
        let txid3 = handle.submit_transaction(tx3).await.unwrap();

        // Get best transactions
        let txs = handle.best_transactions().await.unwrap();
        assert_eq!(txs.len(), 3);

        // Use trait interface (like block assembly would)
        let mut iter: Box<dyn BestTransactions> = Box::new(BestTransactionsIterator::new(txs));
        while let Some((txid, _tx)) = iter.next() {
            if txid == txid2 {
                iter.mark_invalid(txid);
            }
        }

        // Get marked invalid through trait method
        let invalid = iter.marked_invalid();
        assert_eq!(invalid.len(), 1);
        assert!(invalid.contains(&txid2));

        // Remove invalid transactions
        let removed = handle.remove_transactions(invalid).await.unwrap();
        assert!(!removed.is_empty());

        // Verify tx2 removed, others remain
        assert!(!handle.contains(txid2).await.unwrap());
        assert!(handle.contains(txid1).await.unwrap());
        assert!(handle.contains(txid3).await.unwrap());
        assert_eq!(handle.stats().await.unwrap().mempool_size(), 2);
    }

    #[tokio::test]
    async fn test_duplicate_transaction_idempotent() {
        let (handle, _storage) = setup_mempool().await;

        let tx = create_test_snark_tx_with_seq_no(1, 0);
        let tx_clone = tx.clone();

        let txid1 = handle.submit_transaction(tx).await.unwrap();
        let txid2 = handle.submit_transaction(tx_clone).await.unwrap();

        assert_eq!(txid1, txid2, "Same tx should have same txid");
        assert_eq!(handle.stats().await.unwrap().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_chain_update_removes_expired() {
        let (handle, _storage) = setup_mempool().await;

        // Add tx that expires at slot 150
        let tx_expires = create_test_tx_with_expiry(1, Some(50), Some(150), 0);
        // Add tx with no expiry
        let tx_no_expiry = create_test_tx_with_expiry(2, None, None, 0);

        let txid_expires = handle.submit_transaction(tx_expires).await.unwrap();
        let txid_no_expiry = handle.submit_transaction(tx_no_expiry).await.unwrap();

        assert_eq!(handle.stats().await.unwrap().mempool_size(), 2);

        // Chain moves to slot 160 (past expiry)
        handle
            .chain_update(create_test_block_commitment(160))
            .await
            .unwrap();

        // Expired tx removed, non-expiring tx remains
        assert!(!handle.contains(txid_expires).await.unwrap());
        assert!(handle.contains(txid_no_expiry).await.unwrap());
        assert_eq!(handle.stats().await.unwrap().mempool_size(), 1);
    }

    #[tokio::test]
    async fn test_remove_cascade_same_account() {
        let (handle, _storage) = setup_mempool().await;

        // Add multiple txs from same account with sequential seq_nos
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_snark_tx_with_seq_no(1, 1);
        let tx3 = create_test_snark_tx_with_seq_no(1, 2);
        // And one from different account
        let tx_other = create_test_snark_tx_with_seq_no(2, 0);

        let txid1 = handle.submit_transaction(tx1).await.unwrap();
        handle.submit_transaction(tx2).await.unwrap();
        handle.submit_transaction(tx3).await.unwrap();
        let txid_other = handle.submit_transaction(tx_other).await.unwrap();

        assert_eq!(handle.stats().await.unwrap().mempool_size(), 4);

        // Remove first tx from account 1 (cascades to all account 1 txs)
        let removed = handle.remove_transactions(vec![txid1]).await.unwrap();

        // All 3 txs from account 1 removed (cascade)
        assert_eq!(removed.len(), 3);

        // Only tx from account 2 remains
        assert_eq!(handle.stats().await.unwrap().mempool_size(), 1);
        assert!(handle.contains(txid_other).await.unwrap());
    }

    #[tokio::test]
    async fn test_transaction_min_slot_validation() {
        let (handle, _storage) = setup_mempool().await;

        // Add tx valid from slot 200 (current tip is 100)
        let tx_future = create_test_tx_with_expiry(1, Some(200), None, 0);

        // Should be rejected at validation time
        let result = handle.submit_transaction(tx_future).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            OLMempoolError::TransactionNotYetValid { .. }
        ));
    }

    #[tokio::test]
    async fn test_chain_tip_reorg() {
        let (handle, _storage) = setup_mempool().await;

        // Add transactions with sequential seq_nos
        let tx1 = create_test_snark_tx_with_seq_no(1, 0);
        let tx2 = create_test_tx_with_expiry(1, None, Some(150), 1);

        let txid1 = handle.submit_transaction(tx1).await.unwrap();
        let txid2 = handle.submit_transaction(tx2).await.unwrap();

        // Chain at slot 100, both txs valid
        assert_eq!(handle.stats().await.unwrap().mempool_size(), 2);

        // Reorg: chain moves to slot 80 (backward)
        handle
            .chain_update(create_test_block_commitment(80))
            .await
            .unwrap();

        // Both still valid at slot 80
        assert!(handle.contains(txid1).await.unwrap());
        assert!(handle.contains(txid2).await.unwrap());

        // Chain moves forward to slot 200
        handle
            .chain_update(create_test_block_commitment(200))
            .await
            .unwrap();

        // tx2 expired (max_slot 150 < current 200)
        assert!(handle.contains(txid1).await.unwrap());
        assert!(!handle.contains(txid2).await.unwrap());
        assert_eq!(handle.stats().await.unwrap().mempool_size(), 1);
    }
}
