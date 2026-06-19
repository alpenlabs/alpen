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
#[cfg(feature = "prover")]
use strata_asm_params::SubprotocolInstance;
use strata_config::{
    BitcoindConfig, BlockAssemblyConfig, Config, SequencerConfig, SequencerRuntimeConfig,
};
#[cfg(feature = "prover")]
use strata_config::{ProverBackend, ProverConfig};
use strata_csm_types::{ClientState, ClientUpdateOutput, L1Status};
use strata_identifiers::Epoch;
use strata_node_context::NodeContext;
use strata_ol_params::OLParams;
#[cfg(feature = "prover")]
use strata_predicate::{PredicateKey, PredicateTypeId};
use strata_primitives::{L1BlockCommitment, OLBlockCommitment};
#[cfg(feature = "prover")]
use strata_proofimpl_predicate_keys::Sp1Groth16PredicateKey;
#[cfg(feature = "prover")]
use strata_proofimpl_predicate_keys::{NativeCheckpointPredicateKey, validate_predicate_key};
use strata_status::{OLSyncStatus, OLSyncStatusUpdate, StatusChannel};
use strata_storage::{NodeStorage, create_node_storage};
#[cfg(all(feature = "prover", feature = "sp1"))]
use strata_zkvm_hosts::sp1::checkpoint_host;
use tokio::runtime::Handle;
use tracing::{info, warn};

#[cfg(all(feature = "prover", feature = "sp1"))]
use crate::prover::checkpoint_sp1_host_config;
use crate::{args::*, config::*, errors::*, genesis::init_ol_genesis, init_db};

/// Load config early for logging initialization
pub(crate) fn load_config_early(args: &Args) -> Result<Config, InitError> {
    get_config(args.clone())
}

pub(crate) fn init_storage(config: &Config, handle: Handle) -> Result<Arc<NodeStorage>, InitError> {
    let db = init_db::init_database(&config.client)
        .map_err(|e| InitError::StorageCreation(e.to_string()))?;
    let storage = Arc::new(
        create_node_storage(db, handle).map_err(|e| InitError::StorageCreation(e.to_string()))?,
    );
    Ok(storage)
}

/// Initialize runtime, database, bitcoin client, status channel etc.
pub(crate) fn init_node_context(
    args: &Args,
    config: Config,
    handle: Handle,
) -> Result<NodeContext, InitError> {
    // Load ASM params first so integrated prover compatibility checks can use
    // the same checkpoint predicate source that runtime ASM will enforce.
    let asm_params_path = args
        .asm_params
        .as_ref()
        .ok_or(InitError::MissingAsmParams)?;
    let asm_params = load_asm_params(asm_params_path)?;

    // When the integrated prover is enabled, validate that its backend matches the
    // checkpoint predicate the runtime ASM will enforce.
    #[cfg(feature = "prover")]
    validate_integrated_prover_compatibility(&config, &asm_params, &handle)?;

    let blockasm_config = config
        .sequencer
        .as_ref()
        .map(load_block_assembly_config)
        .transpose()?;

    // Load OL params
    let ol_params_path = args.ol_params.as_ref().ok_or(InitError::MissingOLParams)?;
    let ol_params = load_ol_params(ol_params_path)?;

    // Init storage
    let storage = init_storage(&config, handle.clone())?;

    // Init bitcoin client
    let bitcoin_client = create_bitcoin_rpc_client(&config.bitcoind)?;

    // Init status channel
    let status_channel = init_status_channel(&asm_params, &storage)?;

    let nodectx = NodeContext::new(
        handle,
        config,
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
    let override_strs = args.get_all_overrides()?;

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
    env_args.apply_to_config(&mut config);

    validate_config(config)
}

fn validate_config(config: Config) -> Result<Config, InitError> {
    if config.client.is_sequencer
        && config
            .client
            .admin_rpc_bearer_token
            .as_ref()
            .is_none_or(|token| token.expose_secret().is_empty())
    {
        return Err(InitError::MalformedConfig(ConfigError::InvalidOverride {
            override_str: "client.admin_rpc_bearer_token must be set and non-empty".to_string(),
        }));
    }

    if config.client.is_sequencer
        && config
            .client
            .submit_rpc_bearer_token
            .as_ref()
            .is_none_or(|token| token.expose_secret().is_empty())
    {
        return Err(InitError::MalformedConfig(ConfigError::InvalidOverride {
            override_str: "client.submit_rpc_bearer_token must be set and non-empty".to_string(),
        }));
    }

    if config.client.is_sequencer && config.client.rpc_port == config.client.admin_rpc_port {
        return Err(InitError::MalformedConfig(ConfigError::InvalidOverride {
            override_str: "client.admin_rpc_port must differ from client.rpc_port".to_string(),
        }));
    }

    if config.client.is_sequencer && config.client.rpc_port == config.client.submit_rpc_port {
        return Err(InitError::MalformedConfig(ConfigError::InvalidOverride {
            override_str: "client.submit_rpc_port must differ from client.rpc_port".to_string(),
        }));
    }

    if config.client.is_sequencer && config.client.admin_rpc_port == config.client.submit_rpc_port {
        return Err(InitError::MalformedConfig(ConfigError::InvalidOverride {
            override_str: "client.submit_rpc_port must differ from client.admin_rpc_port"
                .to_string(),
        }));
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

#[cfg(feature = "prover")]
fn validate_integrated_prover_compatibility(
    config: &Config,
    asm_params: &AsmParams,
    handle: &Handle,
) -> Result<(), InitError> {
    let checkpoint_predicate = checkpoint_predicate_from_asm_params(asm_params)?;
    let checkpoint_predicate_type = checkpoint_predicate_type(checkpoint_predicate)?;
    let expected_backend = expected_backend_for_checkpoint_predicate(checkpoint_predicate_type)?;

    // When the prover is not configured, validate that the checkpoint predicate
    // does not require real proofs (e.g. Sp1Groth16 needs a prover to produce them).
    let Some(prover_config) = config.prover.as_ref() else {
        if expected_backend.is_some() {
            return Err(InitError::InvalidProverConfig(format!(
                "checkpoint_predicate is {checkpoint_predicate_type} which requires a prover, \
                 but no [prover] section is configured"
            )));
        }
        return Ok(());
    };

    if let Some(expected_backend) = expected_backend
        && prover_config.backend != expected_backend
    {
        return Err(InitError::InvalidProverConfig(format!(
            "prover backend/predicate mismatch: config.prover.backend={:?}, \
             checkpoint_predicate={checkpoint_predicate_type} expects {expected_backend:?}",
            prover_config.backend,
        )));
    }

    validate_checkpoint_predicate_key_for_backend(checkpoint_predicate, prover_config, handle)?;

    Ok(())
}

#[cfg(feature = "prover")]
fn checkpoint_predicate_from_asm_params(
    asm_params: &AsmParams,
) -> Result<&PredicateKey, InitError> {
    let checkpoint_subprotocol = asm_params
        .subprotocols
        .iter()
        .find_map(|instance| match instance {
            SubprotocolInstance::Checkpoint(cfg) => Some(cfg),
            _ => None,
        })
        .ok_or_else(|| {
            InitError::InvalidProverConfig(
                "AsmParams missing Checkpoint subprotocol; cannot validate integrated prover config"
                    .to_string(),
            )
        })?;

    Ok(&checkpoint_subprotocol.checkpoint_predicate)
}

#[cfg(feature = "prover")]
fn checkpoint_predicate_type(predicate: &PredicateKey) -> Result<PredicateTypeId, InitError> {
    let checkpoint_predicate_id = predicate.id();
    PredicateTypeId::try_from(checkpoint_predicate_id).map_err(|e| {
        InitError::InvalidProverConfig(format!(
            "invalid AsmParams checkpoint predicate type id {checkpoint_predicate_id}: {e}"
        ))
    })
}

#[cfg(feature = "prover")]
fn expected_backend_for_checkpoint_predicate(
    checkpoint_predicate_type: PredicateTypeId,
) -> Result<Option<ProverBackend>, InitError> {
    match checkpoint_predicate_type {
        // SP1 checkpoint predicates require SP1 proofs.
        PredicateTypeId::Sp1Groth16 => Ok(Some(ProverBackend::Sp1)),
        // Bip340Schnorr proofs are only produced by the native host's deterministic
        // signing key (functional-test setup), so require the native backend.
        PredicateTypeId::Bip340Schnorr => Ok(Some(ProverBackend::Native)),
        // Other predicate types (including AlwaysAccept/NeverAccept) are not
        // supported by the integrated prover: AlwaysAccept ignores witness bytes
        // and so doesn't need a prover at all, while NeverAccept can't be
        // satisfied by any prover.
        _ => Err(InitError::InvalidProverConfig(format!(
            "unsupported checkpoint predicate for integrated prover: {checkpoint_predicate_type}"
        ))),
    }
}

#[cfg(feature = "prover")]
fn validate_checkpoint_predicate_key_for_backend(
    checkpoint_predicate: &PredicateKey,
    prover_config: &ProverConfig,
    handle: &Handle,
) -> Result<(), InitError> {
    match prover_config.backend {
        ProverBackend::Native => {
            validate_predicate_key(checkpoint_predicate, &NativeCheckpointPredicateKey)
        }
        ProverBackend::Sp1 => {
            let provider = checkpoint_sp1_predicate_key_provider(prover_config, handle)?;
            validate_predicate_key(checkpoint_predicate, &provider)
        }
    }
    .map_err(|e| {
        InitError::InvalidProverConfig(format!(
            "checkpoint predicate key does not match configured prover backend {:?}: {e}",
            prover_config.backend
        ))
    })
}

#[cfg(all(feature = "prover", feature = "sp1"))]
fn checkpoint_sp1_predicate_key_provider(
    prover_config: &ProverConfig,
    handle: &Handle,
) -> Result<Sp1Groth16PredicateKey, InitError> {
    use zkaleido::ZkVmExecutor;

    let sp1_config = checkpoint_sp1_host_config(prover_config);
    let host = handle.block_on(checkpoint_host(sp1_config));
    Ok(Sp1Groth16PredicateKey::new(host.program_id().0))
}

#[cfg(all(feature = "prover", not(feature = "sp1")))]
fn checkpoint_sp1_predicate_key_provider(
    _prover_config: &ProverConfig,
    _handle: &Handle,
) -> Result<Sp1Groth16PredicateKey, InitError> {
    Err(InitError::InvalidProverConfig(
        "config.prover.backend=sp1 requires building `strata` with the `sp1` feature".to_string(),
    ))
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

    // TODO(STR-3694): remove this
    if config.network != Network::Regtest {
        warn!("network not set to regtest, ignoring");
    }
    Ok(btc_rpc.into())
}

/// Status channel initialization
fn init_status_channel(
    asm_params: &AsmParams,
    storage: &NodeStorage,
) -> Result<Arc<StatusChannel>, InitError> {
    let gen_l1 = asm_params.anchor.block;
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

/// Ensures client state and OL genesis, updates status channel.
pub(crate) fn ensure_genesis(
    storage: &NodeStorage,
    ol_params: &OLParams,
    status_channel: &StatusChannel,
) -> Result<(), InitError> {
    let olgen_out = ensure_ol_genesis(storage, ol_params)?;
    let (l1blk, client_state) = ensure_client_state_genesis(storage, ol_params)?;

    // Create sync status and update status channel
    let sync_status = OLSyncStatus::new(
        olgen_out.tip_blk,
        olgen_out.epoch,
        olgen_out.is_terminal,
        client_state.get_last_epoch().unwrap_or_default(),
        client_state.get_last_epoch().unwrap_or_default(),
        client_state.get_declared_final_epoch().unwrap_or_default(),
        l1blk,
    );
    status_channel.update_ol_sync_status(OLSyncStatusUpdate::new(sync_status));

    Ok(())
}

/// Ensures client state genesis.
fn ensure_client_state_genesis(
    storage: &NodeStorage,
    ol_params: &OLParams,
) -> Result<(L1BlockCommitment, ClientState), InitError> {
    // Check for client state genesis
    let csman = storage.client_state();
    let recent_state = csman
        .fetch_most_recent_state()
        .map_err(|e| InitError::StorageCreation(e.to_string()))?;

    match recent_state {
        None => {
            // Create and insert init client state into db.
            let init_state = ClientState::default();
            let l1blk = ol_params.last_l1_block;
            let update = ClientUpdateOutput::new_state(init_state.clone());
            csman.put_update_blocking(&l1blk, update.clone())?;
            Ok((l1blk, update.state().clone()))
        }
        Some(s) => Ok(s),
    }
}

/// OL Chain tip information after ensuring genesis.
#[derive(Clone, Debug)]
pub(crate) struct EnsureOLGenesisOutput {
    pub(crate) epoch: Epoch,
    pub(crate) tip_blk: OLBlockCommitment,
    pub(crate) is_terminal: bool,
}

/// Ensures OL genesis. Returns the tip block commitment and a boolean indicating if it is terminal.
fn ensure_ol_genesis(
    storage: &NodeStorage,
    ol_params: &OLParams,
) -> Result<EnsureOLGenesisOutput, InitError> {
    match storage.ol_block().get_canonical_block_at_blocking(0)? {
        None => {
            info!("No canonical block found at slot 0, doing OL genesis");
            let commitment = init_ol_genesis(ol_params, storage)
                .inspect(|blkid| info!(%blkid, "Done genesis with block"))
                .map_err(|e| InitError::StorageCreation(e.to_string()))?;
            Ok(EnsureOLGenesisOutput {
                epoch: 0,
                tip_blk: commitment,
                is_terminal: true,
            })
        }
        Some(commitment) => {
            info!(%commitment, "Genesis block found, no need to do OL genesis");
            // Get tip
            let tip = storage
                .ol_block()
                .get_canonical_tip_blocking()?
                .expect("tip commitment is expected after genesis");
            let blk = storage
                .ol_block()
                .get_block_data_blocking(*tip.blkid())?
                .expect("tip block expected after genesis");
            Ok(EnsureOLGenesisOutput {
                epoch: blk.header().epoch(),
                tip_blk: tip,
                is_terminal: blk.header().is_terminal(),
            })
        }
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

    #[cfg(feature = "prover")]
    use strata_config::ProverBackend;
    use strata_config::{Config, SequencerConfig};
    #[cfg(feature = "prover")]
    use strata_predicate::PredicateTypeId;

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

    fn fullnode_config() -> Config {
        toml::from_str(
            r#"
            [bitcoind]
            rpc_url = "http://localhost:18332"
            rpc_user = "alpen"
            rpc_password = "alpen"
            network = "regtest"

            [client]
            rpc_host = "0.0.0.0"
            rpc_port = 8432
            admin_rpc_host = "127.0.0.1"
            admin_rpc_port = 8432
            submit_rpc_host = "127.0.0.1"
            submit_rpc_port = 8435
            l2_blocks_fetch_limit = 1_000
            datadir = "/path/to/data/directory"
            db_retry_count = 5

            [btcio.reader]
            client_poll_dur_ms = 200

            [btcio.writer]
            write_poll_dur_ms = 200
            fee_policy = "mempool"
            mempool_base_url = "https://mempool.space/signet"
            reveal_amount = 100
            bundle_interval_ms = 1_000

            [btcio.broadcaster]
            poll_interval_ms = 1_000

            "#,
        )
        .unwrap()
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
                ol_block_time_ms = 5_000

                [fee_model]
                prover_fee_per_gas_wei = 15
                da_overhead_multiplier_bps = 10_000
                ol_overhead_wei = 0
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

    #[test]
    fn validate_config_allows_fullnode_without_admin_token() {
        let config = fullnode_config();

        super::validate_config(config).unwrap();
    }

    #[test]
    fn validate_config_rejects_sequencer_without_admin_token() {
        let mut config = fullnode_config();
        config.client.is_sequencer = true;
        config.client.submit_rpc_bearer_token = Some("test-submit-token".to_string().into());
        config.sequencer = Some(SequencerConfig::default());

        let error = super::validate_config(config).unwrap_err();
        assert!(matches!(error, InitError::MalformedConfig(_)));
    }

    #[test]
    fn validate_config_rejects_sequencer_without_submit_token() {
        let mut config = fullnode_config();
        config.client.is_sequencer = true;
        config.client.admin_rpc_bearer_token = Some("test-token".to_string().into());
        config.sequencer = Some(SequencerConfig::default());

        let error = super::validate_config(config).unwrap_err();
        assert!(matches!(error, InitError::MalformedConfig(_)));
    }

    #[test]
    fn validate_config_rejects_same_public_and_admin_rpc_port_for_sequencer() {
        let mut config = fullnode_config();
        config.client.is_sequencer = true;
        config.client.admin_rpc_bearer_token = Some("test-token".to_string().into());
        config.client.submit_rpc_bearer_token = Some("test-submit-token".to_string().into());
        config.sequencer = Some(SequencerConfig::default());

        let error = super::validate_config(config).unwrap_err();
        assert!(matches!(error, InitError::MalformedConfig(_)));
    }

    #[test]
    fn validate_config_rejects_same_public_and_submit_rpc_port_for_sequencer() {
        let mut config = fullnode_config();
        config.client.is_sequencer = true;
        config.client.admin_rpc_port = 8434;
        config.client.submit_rpc_port = config.client.rpc_port;
        config.client.admin_rpc_bearer_token = Some("test-token".to_string().into());
        config.client.submit_rpc_bearer_token = Some("test-submit-token".to_string().into());
        config.sequencer = Some(SequencerConfig::default());

        let error = super::validate_config(config).unwrap_err();
        assert!(matches!(error, InitError::MalformedConfig(_)));
    }

    #[test]
    fn validate_config_rejects_same_admin_and_submit_rpc_port_for_sequencer() {
        let mut config = fullnode_config();
        config.client.is_sequencer = true;
        config.client.admin_rpc_port = 8434;
        config.client.submit_rpc_port = config.client.admin_rpc_port;
        config.client.admin_rpc_bearer_token = Some("test-token".to_string().into());
        config.client.submit_rpc_bearer_token = Some("test-submit-token".to_string().into());
        config.sequencer = Some(SequencerConfig::default());

        let error = super::validate_config(config).unwrap_err();
        assert!(matches!(error, InitError::MalformedConfig(_)));
    }

    #[cfg(feature = "prover")]
    #[test]
    fn accepts_matching_backend_for_sp1_predicate() {
        let result =
            super::expected_backend_for_checkpoint_predicate(PredicateTypeId::Sp1Groth16).unwrap();
        assert_eq!(result, Some(ProverBackend::Sp1));
    }

    #[cfg(feature = "prover")]
    #[test]
    fn requires_native_backend_for_bip340_schnorr_predicate() {
        let result =
            super::expected_backend_for_checkpoint_predicate(PredicateTypeId::Bip340Schnorr)
                .unwrap();
        assert_eq!(result, Some(ProverBackend::Native));
    }

    #[cfg(feature = "prover")]
    #[test]
    fn rejects_always_accept_predicate_for_integrated_prover() {
        let err = super::expected_backend_for_checkpoint_predicate(PredicateTypeId::AlwaysAccept)
            .unwrap_err();
        assert!(matches!(err, InitError::InvalidProverConfig(_)));
    }

    #[cfg(feature = "prover")]
    #[test]
    fn rejects_unsupported_predicate_for_integrated_prover() {
        let err = super::expected_backend_for_checkpoint_predicate(PredicateTypeId::NeverAccept)
            .unwrap_err();
        assert!(matches!(err, InitError::InvalidProverConfig(_)));
    }

    #[cfg(feature = "prover")]
    #[test]
    fn accepts_matching_checkpoint_predicate_key_provider() {
        use strata_proofimpl_predicate_keys::{
            NativeCheckpointPredicateKey, PredicateKeyProvider, validate_expected_predicate_key,
        };

        let predicate = NativeCheckpointPredicateKey.predicate_key().unwrap();

        validate_expected_predicate_key(&predicate, &predicate).unwrap();
    }

    #[cfg(feature = "prover")]
    #[test]
    fn rejects_mismatched_checkpoint_predicate_key_provider() {
        use strata_predicate::PredicateKey;
        use strata_proofimpl_predicate_keys::{
            NativeCheckpointPredicateKey, PredicateKeyProvider, validate_expected_predicate_key,
        };

        let configured = NativeCheckpointPredicateKey.predicate_key().unwrap();
        let expected = PredicateKey::new(PredicateTypeId::Bip340Schnorr, vec![0u8; 32]);

        let err = validate_expected_predicate_key(&configured, &expected).unwrap_err();

        assert!(err.to_string().contains("predicate key mismatch"));
    }
}
