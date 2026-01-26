use std::sync::Arc;

use bitcoind_async_client::Client;
use strata_config::Config;
use strata_params::Params;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::{TaskExecutor, TaskManager};
use tokio::runtime::Runtime;

/// Contains resources needed to run node services.
#[expect(
    missing_debug_implementations,
    reason = "Not all attributes have debug"
)]
pub struct NodeContext {
    runtime: Runtime,
    executor: Arc<TaskExecutor>,
    config: Config,
    params: Arc<Params>,
    task_manager: TaskManager,
    storage: Arc<NodeStorage>,
    bitcoin_client: Arc<Client>,
    status_channel: Arc<StatusChannel>,
}

impl NodeContext {
    pub fn new(
        runtime: Runtime,
        config: Config,
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        bitcoin_client: Arc<Client>,
        status_channel: Arc<StatusChannel>,
    ) -> Self {
        let task_manager = TaskManager::new(runtime.handle().clone());
        let executor = task_manager.create_executor();
        Self {
            runtime,
            executor: Arc::new(executor),
            config,
            params,
            task_manager,
            storage,
            bitcoin_client,
            status_channel,
        }
    }

    pub fn executor(&self) -> &Arc<TaskExecutor> {
        &self.executor
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn params(&self) -> &Arc<Params> {
        &self.params
    }

    pub fn task_manager(&self) -> &TaskManager {
        &self.task_manager
    }

    pub fn storage(&self) -> &Arc<NodeStorage> {
        &self.storage
    }

    pub fn bitcoin_client(&self) -> &Arc<Client> {
        &self.bitcoin_client
    }

    pub fn status_channel(&self) -> &Arc<StatusChannel> {
        &self.status_channel
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn into_parts(self) -> (Runtime, TaskManager, CommonContext) {
        (
            self.runtime,
            self.task_manager,
            CommonContext {
                executor: self.executor,
                params: self.params,
                config: self.config,
                storage: self.storage,
                status_channel: self.status_channel,
            },
        )
    }
}

/// Common items that all services can use
#[expect(
    missing_debug_implementations,
    reason = "Not all attributes have debug implemented"
)]
pub struct CommonContext {
    executor: Arc<TaskExecutor>,
    params: Arc<Params>,
    config: Config,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
}

impl CommonContext {
    pub fn executor(&self) -> &Arc<TaskExecutor> {
        &self.executor
    }

    pub fn params(&self) -> &Arc<Params> {
        &self.params
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn storage(&self) -> &Arc<NodeStorage> {
        &self.storage
    }

    pub fn status_channel(&self) -> &Arc<StatusChannel> {
        &self.status_channel
    }
}
