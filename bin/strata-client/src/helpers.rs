use std::{fs, path::Path, sync::Arc, time::Duration};

use alloy_rpc_types::engine::JwtSecret;
use bitcoin::{Address, Network};
use bitcoind_async_client::{traits::Wallet, Client};
use format_serde_error::SerdeError;
use strata_config::Config;
use strata_evmexec::{
    engine::RpcExecEngineCtl, evm_genesis_block_hash, fetch_init_fork_choice_state, EngineRpcClient,
};
use strata_primitives::{
    l1::L1Status,
    params::{Params, RollupParams, SyncParams},
};
use strata_state::csm_status::CsmStatus;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tokio::runtime::Handle;
use tracing::*;

use crate::{
    args::{apply_override, parse_override, Args, EnvArgs},
    errors::{ConfigError, InitError},
    network,
};

pub fn get_config(args: Args) -> Result<Config, InitError> {
    // First load from config file.
    let mut config_toml = load_configuration(args.config.as_ref())?;

    // Extend overrides from env.
    let env_args = EnvArgs::from_env();
    let mut override_strs = env_args.get_overrides();

    // Extend overrides from args.
    override_strs.extend_from_slice(&args.get_overrides()?);

    // Parse overrides.
    let overrides = override_strs
        .iter()
        .map(|o| parse_override(o))
        .collect::<Result<Vec<_>, ConfigError>>()?;

    // Apply overrides to toml table.
    let table = config_toml
        .as_table_mut()
        .ok_or(ConfigError::TraverseNonTableAt("".to_string()))?;

    for (path, val) in overrides {
        apply_override(&path, val, table)?;
    }

    // Convert back to Config.
    config_toml
        .try_into::<Config>()
        .map_err(|e| InitError::Anyhow(e.into()))
        .and_then(validate_config)
}

/// Does any extra validations that need to be done for `Config` which are not enforced by type.
fn validate_config(config: Config) -> Result<Config, InitError> {
    // Check if the client is not running as sequencer then has sync endpoint.
    if !config.client.is_sequencer && config.client.sync_endpoint.is_none() {
        return Err(InitError::Anyhow(anyhow::anyhow!("Missing sync_endpoint")));
    }
    Ok(config)
}

fn load_configuration(path: &Path) -> Result<toml::Value, InitError> {
    let config_str = fs::read_to_string(path)?;
    toml::from_str(&config_str).map_err(|e| InitError::Anyhow(e.into()))
}

pub fn load_jwtsecret(path: &Path) -> Result<JwtSecret, InitError> {
    let secret = fs::read_to_string(path)?;
    Ok(JwtSecret::from_hex(secret)?)
}

/// Resolves the rollup params file to use, possibly from a path, and validates
/// it to ensure it passes sanity checks.
pub fn resolve_and_validate_params(
    path: Option<&Path>,
    config: &Config,
) -> Result<Arc<Params>, InitError> {
    let rollup_params = resolve_rollup_params(path)?;
    rollup_params.check_well_formed()?;

    let params = Params {
        rollup: rollup_params,
        run: SyncParams {
            // FIXME these shouldn't be configurable here
            l1_follow_distance: config.sync.l1_follow_distance,
            client_checkpoint_interval: config.sync.client_checkpoint_interval,
            l2_blocks_fetch_limit: config.client.l2_blocks_fetch_limit,
        },
    }
    .into();
    Ok(params)
}

/// Resolves the rollup params file to use, possibly from a path.
pub fn resolve_rollup_params(path: Option<&Path>) -> Result<RollupParams, InitError> {
    // If a path is set from arg load that.
    if let Some(p) = path {
        return load_rollup_params(p);
    }

    // Otherwise check from envvar.
    if let Some(p) = network::get_envvar_params()? {
        return Ok(p);
    }

    // *Otherwise*, use the fallback.
    Ok(network::get_default_rollup_params()?)
}

fn load_rollup_params(path: &Path) -> Result<RollupParams, InitError> {
    let json = fs::read_to_string(path)?;
    let rollup_params =
        serde_json::from_str::<RollupParams>(&json).map_err(|err| SerdeError::new(json, err))?;
    Ok(rollup_params)
}

// TODO: remove this after builder is done
pub fn create_bitcoin_rpc_client(config: &Config) -> anyhow::Result<Arc<Client>> {
    // Set up Bitcoin client RPC.
    let bitcoind_url = format!("http://{}", config.bitcoind.rpc_url);
    let btc_rpc = Client::new(
        bitcoind_url,
        config.bitcoind.rpc_user.clone(),
        config.bitcoind.rpc_password.clone(),
        config.bitcoind.retry_count,
        config.bitcoind.retry_interval,
    )
    .map_err(anyhow::Error::from)?;

    // TODO remove this
    if config.bitcoind.network != Network::Regtest {
        warn!("network not set to regtest, ignoring");
    }
    Ok(btc_rpc.into())
}

// initializes the status bundle that we can pass around cheaply for status/metrics
pub fn init_status_channel(storage: &NodeStorage) -> anyhow::Result<StatusChannel> {
    // init client state
    let csman = storage.client_state();
    let (cur_state_idx, cur_state) = csman
        .get_most_recent_state_blocking()
        .ok_or(InitError::MissingInitClientState)?;

    // init the CsmStatus
    let mut status = CsmStatus::default();
    status.set_last_sync_ev_idx(cur_state_idx);
    status.update_from_client_state(&cur_state);

    let l1_status = L1Status {
        ..Default::default()
    };

    // TODO avoid clone, change status channel to use arc
    Ok(StatusChannel::new(
        cur_state.as_ref().clone(),
        l1_status,
        None,
    ))
}

pub fn init_engine_controller(
    config: &Config,
    params: &Params,
    storage: &NodeStorage,
    handle: &Handle,
    from_genesis: bool,
) -> anyhow::Result<Arc<RpcExecEngineCtl<EngineRpcClient>>> {
    let reth_jwtsecret = load_jwtsecret(&config.exec.reth.secret)?;
    let client = EngineRpcClient::from_url_secret(
        &format!("http://{}", &config.exec.reth.rpc_url),
        reth_jwtsecret,
    );

    let initial_fcs = if from_genesis {
        evm_genesis_block_hash(params.rollup())
    } else {
        fetch_init_fork_choice_state(storage, params.rollup())?
    };

    let eng_ctl = strata_evmexec::engine::RpcExecEngineCtl::new(
        client,
        initial_fcs,
        handle.clone(),
        storage.l2().clone(),
    );
    let eng_ctl = Arc::new(eng_ctl);
    Ok(eng_ctl)
}

/// Get an address controlled by sequencer's bitcoin wallet
pub async fn generate_sequencer_address(
    bitcoin_client: &Client,
    timeout: u64,
    poll_interval: u64,
) -> anyhow::Result<Address> {
    let mut last_err = None;
    tokio::time::timeout(Duration::from_secs(timeout), async {
        loop {
            match bitcoin_client.get_new_address().await {
                Ok(address) => return address,
                Err(err) => {
                    warn!(err = ?err, "failed to generate address");
                    last_err.replace(err);
                }
            }
            // Sleep for a while just to prevent excessive continuous calls in short time
            tokio::time::sleep(Duration::from_millis(poll_interval)).await;
        }
    })
    .await
    .map_err(|_| match last_err {
        None => anyhow::Error::msg("failed to generate address; timeout"),
        Some(client_error) => {
            anyhow::Error::from(client_error).context("failed to generate address")
        }
    })
}
