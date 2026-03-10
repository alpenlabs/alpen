//! Node context initialization and configuration loading.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use bitcoin::Network;
use bitcoind_async_client::{Auth, Client};
use format_serde_error::SerdeError;
use strata_asm_params::AsmParams;
use strata_config::{BitcoindConfig, BlockAssemblyConfig, Config};
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
    let blockasm_config = config
        .client
        .is_sequencer
        .then(|| load_block_assembly_config(params_path))
        .transpose()?;

    // Load ASM params
    let asm_params_path = args
        .asm_params
        .as_ref()
        .ok_or(InitError::MissingAsmParams)?;
    let asm_params = load_asm_params(asm_params_path)?;

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
        blockasm_config,
        Arc::new(asm_params),
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

    let env_args = EnvArgs::from_env()?;
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

fn resolve_block_assembly_config_path(params_path: &Path) -> PathBuf {
    let stem = params_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("params");
    let file_name = match params_path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => format!("{stem}.blockasm.{ext}"),
        None => format!("{stem}.blockasm"),
    };
    params_path.with_file_name(file_name)
}

fn resolve_block_assembly_config_fallback_path(params_path: &Path) -> PathBuf {
    params_path.with_file_name("blockasm.json")
}

pub(crate) fn load_block_assembly_config(
    params_path: &Path,
) -> Result<Arc<BlockAssemblyConfig>, InitError> {
    let path = resolve_block_assembly_config_path(params_path);
    let fallback_path = resolve_block_assembly_config_fallback_path(params_path);
    let json = match fs::read_to_string(&path) {
        Ok(json) => json,
        Err(err) if err.kind() == io::ErrorKind::NotFound => fs::read_to_string(fallback_path)?,
        Err(err) => return Err(err.into()),
    };
    let config = serde_json::from_str::<serde_json::Value>(&json)
        .map_err(|err| InitError::UnparsableBlockAssemblyConfigFile(SerdeError::new(json, err)))?;
    let ol_block_time_ms = config
        .get("ol_block_time_ms")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            InitError::InvalidOlBlockTimeMs(
                "block assembly config ol_block_time_ms must be a positive integer".to_string(),
            )
        })?;

    if ol_block_time_ms == 0 {
        return Err(InitError::InvalidOlBlockTimeMs(
            "block assembly config ol_block_time_ms must be greater than 0".to_string(),
        ));
    }

    Ok(Arc::new(BlockAssemblyConfig::new(Duration::from_millis(
        ol_block_time_ms,
    ))))
}

fn load_asm_params(path: &Path) -> Result<AsmParams, InitError> {
    let json = fs::read_to_string(path)?;
    let asm_params =
        serde_json::from_str::<AsmParams>(&json).map_err(|err| SerdeError::new(json, err))?;
    Ok(asm_params)
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

#[cfg(test)]
mod tests {
    use std::{
        env::temp_dir,
        fs,
        path::PathBuf,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{
        load_block_assembly_config, resolve_block_assembly_config_fallback_path,
        resolve_block_assembly_config_path,
    };
    use crate::errors::InitError;

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after unix epoch")
            .as_nanos();
        temp_dir().join(format!("strata-context-tests-{nanos}"))
    }

    #[test]
    fn resolve_block_assembly_config_path_uses_sidecar_name() {
        let params_path = PathBuf::from("/tmp/params.json");
        let config_path = resolve_block_assembly_config_path(&params_path);

        assert_eq!(config_path, PathBuf::from("/tmp/params.blockasm.json"));
    }

    #[test]
    fn load_block_assembly_config_reads_positive_block_time() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();

        let params_path = temp_dir.join("params.json");
        fs::write(&params_path, "{}").unwrap();
        fs::write(
            resolve_block_assembly_config_path(&params_path),
            r#"{"ol_block_time_ms": 5000}"#,
        )
        .unwrap();

        let config = load_block_assembly_config(&params_path).unwrap();
        assert_eq!(config.ol_block_time(), Duration::from_millis(5_000));

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn load_block_assembly_config_rejects_zero_block_time() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();

        let params_path = temp_dir.join("params.json");
        fs::write(&params_path, "{}").unwrap();
        fs::write(
            resolve_block_assembly_config_path(&params_path),
            r#"{"ol_block_time_ms": 0}"#,
        )
        .unwrap();

        let error = load_block_assembly_config(&params_path).unwrap_err();
        assert!(matches!(error, InitError::InvalidOlBlockTimeMs(_)));

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn load_block_assembly_config_reads_fallback_path() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();

        let params_path = temp_dir.join("rollup-params.json");
        fs::write(&params_path, "{}").unwrap();
        fs::write(
            resolve_block_assembly_config_fallback_path(&params_path),
            r#"{"ol_block_time_ms": 5000}"#,
        )
        .unwrap();

        let config = load_block_assembly_config(&params_path).unwrap();
        assert_eq!(config.ol_block_time(), Duration::from_millis(5_000));

        fs::remove_dir_all(temp_dir).unwrap();
    }
}
