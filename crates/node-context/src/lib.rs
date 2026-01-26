use std::sync::Arc;

use bitcoind_async_client::Client;
use strata_config::Config;
use strata_params::Params;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::{TaskExecutor, TaskManager};

/// Contains resources needed to run node services.
#[expect(
    missing_debug_implementations,
    reason = "Not all attributes have debug"
)]
pub struct NodeContext {
    executor: TaskExecutor,
    config: Config,
    params: Arc<Params>,
    task_manager: TaskManager,
    storage: Arc<NodeStorage>,
    bitcoin_client: Arc<Client>,
    status_channel: Arc<StatusChannel>,
}

impl NodeContext {
    pub fn new(
        executor: TaskExecutor,
        config: Config,
        params: Arc<Params>,
        task_manager: TaskManager,
        storage: Arc<NodeStorage>,
        bitcoin_client: Arc<Client>,
        status_channel: Arc<StatusChannel>,
    ) -> Self {
        Self {
            executor,
            config,
            params,
            task_manager,
            storage,
            bitcoin_client,
            status_channel,
        }
    }

    pub fn executor(&self) -> &TaskExecutor {
        &self.executor
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn params(&self) -> &Params {
        &self.params
    }

    pub fn task_manager(&self) -> &TaskManager {
        &self.task_manager
    }

    pub fn storage(&self) -> &NodeStorage {
        &self.storage
    }

    pub fn bitcoin_client(&self) -> &Client {
        &self.bitcoin_client
    }

    pub fn status_channel(&self) -> &StatusChannel {
        &self.status_channel
    }
}
