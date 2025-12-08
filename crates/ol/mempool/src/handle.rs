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
    use strata_storage::create_node_storage;
    use strata_tasks::TaskManager;
    use threadpool::ThreadPool;

    use super::*;
    use crate::{test_utils::create_test_block_commitment, types::OLMempoolConfig};

    #[test]
    fn test_service_closed_error() {
        let err = MempoolHandle::service_closed_error(());
        assert!(matches!(err, OLMempoolError::ServiceClosed(_)));
    }

    #[tokio::test]
    async fn test_launch_with_status_channel() {
        // Test that launch() works correctly with StatusChannel subscription
        let pool = ThreadPool::new(1);
        let test_db = get_test_sled_backend();
        let storage = Arc::new(
            create_node_storage(test_db, pool).expect("Failed to create test NodeStorage"),
        );

        let config = OLMempoolConfig::default();
        let current_tip = create_test_block_commitment(100);

        // Create a minimal StatusChannel
        let client_state = ClientState::new(None, None);
        let l1_block = L1BlockCommitment::from_height_u64(0, L1BlockId::default())
            .expect("Failed to create L1BlockCommitment");
        let l1_status = L1Status::default();
        let status_channel = StatusChannel::new(client_state, l1_block, l1_status, None);

        let task_manager = TaskManager::new(tokio::runtime::Handle::current());
        let texec = task_manager.create_executor();

        // Verify launch succeeds with StatusChannel
        let handle = MempoolHandle::launch(storage, config, current_tip, status_channel, &texec)
            .await
            .expect("Should launch successfully");

        // Verify handle is functional by checking stats
        let stats = handle.stats().await.expect("Should get stats");
        assert_eq!(stats.mempool_size(), 0);
    }
}
