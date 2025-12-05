//! Node context initialization and configuration loading.

use std::{fs, path::Path, sync::Arc};

use bitcoin::Network;
use bitcoind_async_client::Client;
use format_serde_error::SerdeError;
use strata_config::{BitcoindConfig, Config};
use strata_csm_types::L1Status;
use strata_params::{Params, RollupParams, SyncParams};
use strata_status::StatusChannel;
use strata_storage::{NodeStorage, create_node_storage};
use strata_tasks::{TaskExecutor, TaskManager};
use tokio::runtime::Runtime;
use tracing::warn;

use crate::{args::*, config::*, errors::*, init_db};

/// Contains resources needed to run node services.
#[expect(unused, reason = "will be used later")]
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

/// Initialize runtime, database, etc.
pub(crate) fn init_node_context(args: Args) -> Result<NodeContext, InitError> {
    let config = get_config(args.clone())?;
    let params_path = args.rollup_params.ok_or(InitError::MissingRollupParams)?;
    let params = resolve_and_validate_params(&params_path, &config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("strata-rt")
        .build()
        .map_err(InitError::RuntimeBuild)?;

    let task_manager = TaskManager::new(runtime.handle().clone());
    let executor = task_manager.create_executor();

    let db = init_db::init_database(&config.client)
        .map_err(|e| InitError::StorageCreation(e.to_string()))?;
    let pool = threadpool::ThreadPool::with_name("strata-pool".to_owned(), 8);
    let storage = Arc::new(
        create_node_storage(db, pool.clone())
            .map_err(|e| InitError::StorageCreation(e.to_string()))?,
    );

    // Init bitcoin client
    let bitcoin_client = create_bitcoin_rpc_client(&config.bitcoind)?;

    // Init status channel
    let status_channel = init_status_channel(&storage)?;

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

// Config loading and validation

fn get_config(args: Args) -> Result<Config, InitError> {
    let mut config_toml = load_config_from_path(args.config.as_ref())?;

    let env_args = EnvArgs::from_env();
    let mut override_strs = env_args.get_overrides();

    override_strs.extend_from_slice(&args.get_all_overrides()?);

    let overrides = override_strs
        .iter()
        .map(|o| parse_override(o))
        .collect::<Result<Vec<_>, ConfigError>>()?;

    let table = config_toml
        .as_table_mut()
        .ok_or(ConfigError::TraverseNonTableAt {
            key: "<root>".to_string(),
            path: "".to_string(),
        })?;

    for (path, val) in overrides {
        apply_override(&path, val, table)?;
    }

    let config = config_toml
        .try_into::<Config>()
        .map_err(InitError::TomlParse)?;

    validate_config(config)
}

fn validate_config(config: Config) -> Result<Config, InitError> {
    if !config.client.is_sequencer && config.client.sync_endpoint.is_none() {
        return Err(InitError::MissingSyncEndpoint);
    }
    Ok(config)
}

fn load_config_from_path(path: &Path) -> Result<toml::Value, InitError> {
    let config_str = fs::read_to_string(path)?;
    toml::from_str(&config_str).map_err(InitError::TomlParse)
}

fn resolve_and_validate_params(path: &Path, config: &Config) -> Result<Arc<Params>, InitError> {
    let rollup_params = load_rollup_params(path)?;
    rollup_params.check_well_formed()?;

    let params = Params {
        rollup: rollup_params,
        run: SyncParams {
            l1_follow_distance: config.sync.l1_follow_distance,
            client_checkpoint_interval: config.sync.client_checkpoint_interval,
            l2_blocks_fetch_limit: config.client.l2_blocks_fetch_limit,
        },
    }
    .into();
    Ok(params)
}

fn load_rollup_params(path: &Path) -> Result<RollupParams, InitError> {
    let json = fs::read_to_string(path)?;
    let rollup_params =
        serde_json::from_str::<RollupParams>(&json).map_err(|err| SerdeError::new(json, err))?;
    Ok(rollup_params)
}

/// Bitcoin client initialization
fn create_bitcoin_rpc_client(config: &BitcoindConfig) -> Result<Arc<Client>, InitError> {
    let btc_rpc = Client::new(
        config.rpc_url.clone(),
        config.rpc_user.clone(),
        config.rpc_password.clone(),
        config.retry_count,
        config.retry_interval,
    )
    .map_err(|e| InitError::BitcoinClientCreation(e.to_string()))?;

    // TODO remove this
    if config.network != Network::Regtest {
        warn!("network not set to regtest, ignoring");
    }
    Ok(btc_rpc.into())
}

/// Status channel initialization
fn init_status_channel(storage: &NodeStorage) -> Result<Arc<StatusChannel>, InitError> {
    let csman = storage.client_state();
    let (cur_block, cur_state) = csman
        .fetch_most_recent_state()
        .map_err(|e| InitError::StorageCreation(e.to_string()))?
        .ok_or(InitError::MissingInitialState)?;

    let l1_status = L1Status {
        ..Default::default()
    };

    Ok(StatusChannel::new(cur_state, cur_block, l1_status, None).into())
}
