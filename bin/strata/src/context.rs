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
use strata_config::{
    BitcoindConfig, BlockAssemblyConfig, Config, SequencerConfig, SequencerRuntimeConfig,
};
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
        .sequencer
        .as_ref()
        .map(load_block_assembly_config)
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

    let mut config = config_toml
        .try_into::<Config>()
        .map_err(InitError::TomlParse)?;

    populate_sequencer_runtime_config(&mut config, &args)?;

    validate_config(config)
}

fn validate_config(config: Config) -> Result<Config, InitError> {
    if !config.client.is_sequencer && config.client.sync_endpoint.is_none() {
        return Err(InitError::MissingSyncEndpoint);
    }

    if config.client.is_sequencer && config.sequencer.is_none() {
        return Err(InitError::MissingSequencerConfig(PathBuf::from(
            "sequencer.toml",
        )));
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

fn populate_sequencer_runtime_config(config: &mut Config, args: &Args) -> Result<(), InitError> {
    if !config.client.is_sequencer {
        return Ok(());
    }

    let path = resolve_sequencer_config_path(args);
    let runtime_config = load_sequencer_runtime_config(&path)?;

    config.sequencer = Some(runtime_config.sequencer);
    config.epoch_sealing = runtime_config.epoch_sealing;

    Ok(())
}

fn resolve_default_sequencer_config_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("sequencer.toml")
}

fn resolve_sequencer_config_path(args: &Args) -> PathBuf {
    args.sequencer_config
        .clone()
        .unwrap_or_else(|| resolve_default_sequencer_config_path(args.config.as_path()))
}

fn load_sequencer_runtime_config(path: &Path) -> Result<SequencerRuntimeConfig, InitError> {
    let config_str = fs::read_to_string(path).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => InitError::MissingSequencerConfig(path.to_path_buf()),
        _ => InitError::Io(err),
    })?;
    toml::from_str(&config_str).map_err(InitError::UnparsableSequencerConfigFile)
}

fn validate_ol_block_time_ms(ol_block_time_ms: u64) -> Result<(), InitError> {
    if ol_block_time_ms == 0 {
        return Err(InitError::InvalidOlBlockTimeMs(ol_block_time_ms));
    }

    Ok(())
}

pub(crate) fn load_block_assembly_config(
    sequencer_config: &SequencerConfig,
) -> Result<Arc<BlockAssemblyConfig>, InitError> {
    validate_ol_block_time_ms(sequencer_config.ol_block_time_ms)?;

    Ok(Arc::new(BlockAssemblyConfig::new(Duration::from_millis(
        sequencer_config.ol_block_time_ms,
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
        path::{Path, PathBuf},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use strata_config::SequencerConfig;

    use super::{
        load_block_assembly_config, load_sequencer_runtime_config,
        resolve_default_sequencer_config_path,
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
    fn resolve_default_sequencer_config_path_uses_sibling_name() {
        let config_path = resolve_default_sequencer_config_path(Path::new("/tmp/config.toml"));

        assert_eq!(config_path, PathBuf::from("/tmp/sequencer.toml"));
    }

    #[test]
    fn load_sequencer_runtime_config_reads_sequencer_toml() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).unwrap();

        let sequencer_config_path = temp_dir.join("sequencer.toml");
        fs::write(
            &sequencer_config_path,
            r#"
                [sequencer]
                ol_block_time_ms = 5000
            "#,
        )
        .unwrap();

        let runtime_config = load_sequencer_runtime_config(&sequencer_config_path).unwrap();
        let config = load_block_assembly_config(&runtime_config.sequencer).unwrap();
        assert_eq!(config.ol_block_time(), Duration::from_millis(5_000));

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn load_block_assembly_config_rejects_zero_block_time() {
        let error = load_block_assembly_config(&SequencerConfig {
            ol_block_time_ms: 0,
            ..SequencerConfig::default()
        })
        .unwrap_err();
        assert!(matches!(error, InitError::InvalidOlBlockTimeMs(0)));
    }
}
