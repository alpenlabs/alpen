use std::sync::Arc;

use bitcoind_async_client::Client;
use strata_params::Params;
use strata_status::StatusChannel;
use strata_storage::{NodeStorage, create_node_storage};
use strata_tasks::{TaskExecutor, TaskManager};
use tokio::runtime::Runtime;

use crate::{
    Config,
    errors::InitError,
    helpers::{create_bitcoin_rpc_client, init_status_channel},
    init_db,
};

/// Contains stuffs to create various node services.
pub(crate) struct NodeContext {
    pub runtime: Runtime,
    pub config: Config,
    pub params: Arc<Params>,
    pub task_manager: TaskManager,
    pub executor: TaskExecutor,
    pub storage: Arc<NodeStorage>,
    pub bitcoin_client: Arc<Client>,
    pub status_channel: Arc<StatusChannel>,
}

// Initialize runtime, database, etc.
pub(crate) fn init_node_context(
    config: Config,
    params: Arc<Params>,
) -> Result<NodeContext, InitError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("strata-rt")
        .build()
        .expect("init: build rt");

    let task_manager = TaskManager::new(runtime.handle().clone());
    let executor = task_manager.create_executor();

    let db = init_db::init_database(&config.client.datadir, config.client.db_retry_count)?;
    let pool = threadpool::ThreadPool::with_name("strata-pool".to_owned(), 8);
    let storage = Arc::new(create_node_storage(db, pool.clone()).unwrap());

    // Init bitcoin client
    let bitcoin_client = create_bitcoin_rpc_client(&config.bitcoind)?;

    // Init status channel
    let status_channel = init_status_channel(&storage)?.into();

    Ok(NodeContext {
        runtime,
        config,
        params,
        task_manager,
        executor,
        storage,
        bitcoin_client,
        status_channel,
    })
}
