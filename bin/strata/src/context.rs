//! Node context initialization and configuration loading.

use std::{fs, path::Path, sync::Arc};

use bitcoin::Network;
use bitcoind_async_client::{Auth, Client};
use format_serde_error::SerdeError;
use strata_config::{BitcoindConfig, Config};
use strata_csm_types::{ClientState, ClientUpdateOutput, L1Status};
use strata_node_context::NodeContext;
use strata_ol_params::OLParams;
use strata_params::{Params, RollupParams, SyncParams};
use strata_primitives::L1BlockCommitment;
use strata_status::StatusChannel;
use strata_storage::{NodeStorage, create_node_storage};
use tokio::runtime::Handle;
use tracing::warn;

use crate::{args::*, config::*, errors::*, genesis::init_ol_genesis, init_db};

/// Load config early for logging initialization
pub(crate) fn load_config_early(args: &Args) -> Result<Config, InitError> {
    get_config(args.clone())
}

pub(crate) fn init_storage(config: &Config) -> Result<Arc<NodeStorage>, InitError> {
    let db = init_db::init_database(&config.client)
        .map_err(|e| InitError::StorageCreation(e.to_string()))?;
    let pool = threadpool::ThreadPool::with_name("strata-pool".to_owned(), 8);
    let storage = Arc::new(
        create_node_storage(db, pool).map_err(|e| InitError::StorageCreation(e.to_string()))?,
    );
    Ok(storage)
}

/// Initialize runtime, database, bitcoin client, status channel etc.
pub(crate) fn init_node_context(
    args: &Args,
    config: Config,
    handle: Handle,
) -> Result<NodeContext, InitError> {
    // Validate params
    let params_path = args
        .rollup_params
        .as_ref()
        .ok_or(InitError::MissingRollupParams)?;
    let params = resolve_and_validate_params(params_path, &config)?;

    // Load OL params
    let ol_params_path = args.ol_params.as_ref().ok_or(InitError::MissingOLParams)?;
    let ol_params = load_ol_params(ol_params_path)?;

    // Init storage
    let storage = init_storage(&config)?;

    // Init bitcoin client
    let bitcoin_client = create_bitcoin_rpc_client(&config.bitcoind)?;

    // Init status channel
    let status_channel = init_status_channel(params.rollup(), &storage)?;

    let nodectx = NodeContext::new(
        handle,
        config,
        params,
        ol_params.into(),
        storage,
        bitcoin_client,
        status_channel,
    );

    Ok(nodectx)
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

pub(crate) fn resolve_and_validate_params(
    path: &Path,
    config: &Config,
) -> Result<Arc<Params>, InitError> {
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

fn load_ol_params(path: &Path) -> Result<OLParams, InitError> {
    let json = fs::read_to_string(path)?;
    let ol_params =
        serde_json::from_str::<OLParams>(&json).map_err(|err| SerdeError::new(json, err))?;
    Ok(ol_params)
}

/// Bitcoin client initialization
fn create_bitcoin_rpc_client(config: &BitcoindConfig) -> Result<Arc<Client>, InitError> {
    let auth = Auth::UserPass(config.rpc_user.clone(), config.rpc_password.clone());
    let btc_rpc = Client::new(
        config.rpc_url.clone(),
        auth,
        config.retry_count,
        config.retry_interval,
        None,
    )
    .map_err(|e| InitError::BitcoinClientCreation(e.to_string()))?;

    // TODO remove this
    if config.network != Network::Regtest {
        warn!("network not set to regtest, ignoring");
    }
    Ok(btc_rpc.into())
}

/// Status channel initialization
fn init_status_channel(
    params: &RollupParams,
    storage: &NodeStorage,
) -> Result<Arc<StatusChannel>, InitError> {
    let gen_l1 = params.genesis_l1_view.blk;
    let csman = storage.client_state();
    let (cur_block, cur_state) = csman
        .fetch_most_recent_state()
        .map_err(|e| InitError::StorageCreation(e.to_string()))?
        .unwrap_or((gen_l1, ClientState::default()));

    let l1_status = L1Status {
        ..Default::default()
    };

    Ok(StatusChannel::new(cur_state, cur_block, l1_status, None, None).into())
}

pub(crate) fn check_and_init_genesis(
    storage: &NodeStorage,
    ol_params: &OLParams,
) -> Result<(L1BlockCommitment, ClientState), InitError> {
    let csman = storage.client_state();
    let recent_state = csman
        .fetch_most_recent_state()
        .map_err(|e| InitError::StorageCreation(e.to_string()))?;

    match recent_state {
        None => {
            // Initialize OL genesis block and state
            init_ol_genesis(ol_params, storage)
                .map_err(|e| InitError::StorageCreation(e.to_string()))?;

            // Create and insert init client state into db.
            let init_state = ClientState::default();
            let l1blk = ol_params.last_l1_block;
            let update = ClientUpdateOutput::new_state(init_state.clone());
            csman.put_update_blocking(&l1blk, update.clone())?;
            Ok((l1blk, init_state))
        }
        Some(recent_state) => Ok(recent_state),
    }
}
