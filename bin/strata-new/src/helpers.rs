use std::{fs, path::Path, sync::Arc};

use bitcoin::Network;
use bitcoind_async_client::Client;
use format_serde_error::SerdeError;
use strata_config::{BitcoindConfig, Config};
use strata_csm_types::L1Status;
use strata_params::{Params, RollupParams, SyncParams};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tracing::warn;

use crate::{args::*, errors::*};

pub(crate) fn get_config(args: Args) -> Result<Config, InitError> {
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

/// Resolves the rollup params file to use, possibly from a path, and validates
/// it to ensure it passes sanity checks.
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

pub(crate) fn create_bitcoin_rpc_client(config: &BitcoindConfig) -> anyhow::Result<Arc<Client>> {
    // Set up Bitcoin client RPC.
    let btc_rpc = Client::new(
        config.rpc_url.clone(),
        config.rpc_user.clone(),
        config.rpc_password.clone(),
        config.retry_count,
        config.retry_interval,
    )
    .map_err(anyhow::Error::from)?;

    // TODO remove this
    if config.network != Network::Regtest {
        warn!("network not set to regtest, ignoring");
    }
    Ok(btc_rpc.into())
}

// initializes the status bundle that we can pass around cheaply for status/metrics
pub(crate) fn init_status_channel(storage: &NodeStorage) -> anyhow::Result<StatusChannel> {
    // init client state
    let csman = storage.client_state();
    let (cur_block, cur_state) = csman
        .fetch_most_recent_state()?
        .expect("missing init client state?");

    let l1_status = L1Status {
        ..Default::default()
    };

    // TODO avoid clone, change status channel to use arc
    Ok(StatusChannel::new(cur_state, cur_block, l1_status, None))
}
