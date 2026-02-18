//! Stores contexts that can be passed for initializing and running various strata services
use std::sync::Arc;

use bitcoind_async_client::Client;
use strata_asm_params::AsmParams;
use strata_config::Config;
use strata_ol_params::OLParams;
use strata_params::Params;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::{TaskExecutor, TaskManager};
use tokio::runtime::Handle;

/// Contains resources needed to run node services.
#[expect(
    missing_debug_implementations,
    reason = "Not all attributes have debug"
)]
pub struct NodeContext {
    executor: Arc<TaskExecutor>,
    config: Config,
    params: Arc<Params>,
    asm_params: Arc<AsmParams>,
    ol_params: Arc<OLParams>,
    task_manager: TaskManager,
    storage: Arc<NodeStorage>,
    bitcoin_client: Arc<Client>,
    status_channel: Arc<StatusChannel>,
}

impl NodeContext {
    #[expect(
        clippy::too_many_arguments,
        reason = "Constructor needs all fields to initialize NodeContext"
    )]
    pub fn new(
        handle: Handle,
        config: Config,
        params: Arc<Params>,
        asm_params: Arc<AsmParams>,
        ol_params: Arc<OLParams>,
        storage: Arc<NodeStorage>,
        bitcoin_client: Arc<Client>,
        status_channel: Arc<StatusChannel>,
    ) -> Self {
        let task_manager = TaskManager::new(handle);
        let executor = task_manager.create_executor();
        Self {
            executor: Arc::new(executor),
            config,
            params,
            asm_params,
            ol_params,
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

    pub fn asm_params(&self) -> &Arc<AsmParams> {
        &self.asm_params
    }

    pub fn ol_params(&self) -> &Arc<OLParams> {
        &self.ol_params
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

    pub fn into_parts(self) -> (TaskManager, CommonContext) {
        (
            self.task_manager,
            CommonContext {
                executor: self.executor,
                params: self.params,
                asm_params: self.asm_params,
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
    asm_params: Arc<AsmParams>,
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

    pub fn asm_params(&self) -> &Arc<AsmParams> {
        &self.asm_params
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
