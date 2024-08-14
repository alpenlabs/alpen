#![allow(dead_code)] // TODO: remove this once `Args.network` is used
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::thread;

use anyhow::Context;
use bitcoin::Network;
use config::Config;
use format_serde_error::SerdeError;
use reth_rpc_types::engine::JwtError;
use reth_rpc_types::engine::JwtSecret;
use rockbound::rocksdb;
use thiserror::Error;
use tokio::sync::{broadcast, oneshot, RwLock};
use tracing::*;

use alpen_express_btcio::rpc::traits::L1Client;
use alpen_express_btcio::writer::config::WriterConfig;
use alpen_express_btcio::writer::start_writer_task;
use alpen_express_btcio::writer::DaWriter;
use alpen_express_common::logging;
use alpen_express_consensus_logic::duty::types::{DutyBatch, Identity, IdentityData, IdentityKey};
use alpen_express_consensus_logic::duty::worker::{self as duty_worker};
use alpen_express_consensus_logic::sync_manager;
use alpen_express_consensus_logic::sync_manager::SyncManager;
use alpen_express_db::traits::Database;
use alpen_express_db::traits::SequencerDatabase;
use alpen_express_evmexec::{fork_choice_state_initial, EngineRpcClient};
use alpen_express_primitives::block_credential;
use alpen_express_primitives::buf::Buf32;
use alpen_express_primitives::params::{Params, RollupParams, RunParams};
use alpen_express_rocksdb::sequencer::db::SequencerDB;
use alpen_express_rocksdb::SeqDb;
use alpen_express_rpc_api::{AlpenAdminApiServer, AlpenApiServer};
use alpen_express_rpc_types::L1Status;

use crate::args::Args;

mod args;
mod config;
mod l1_reader;
mod rpc_server;

#[derive(Debug, Error)]
pub enum InitError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("config: {0}")]
    MalformedConfig(#[from] SerdeError),

    #[error("jwt: {0}")]
    MalformedSecret(#[from] JwtError),

    #[error("{0}")]
    Other(String),
}

fn load_configuration(path: &Path) -> Result<Config, InitError> {
    let config_str = fs::read_to_string(path)?;
    let conf = toml::from_str::<Config>(&config_str)
        .map_err(|err| SerdeError::new(config_str.to_string(), err))?;
    Ok(conf)
}

fn load_jwtsecret(path: &Path) -> Result<JwtSecret, InitError> {
    let secret = fs::read_to_string(path)?;
    let jwt_secret = JwtSecret::from_hex(secret)?;

    Ok(jwt_secret)
}

fn load_rollup_params_or_default(path: &Option<PathBuf>) -> Result<RollupParams, InitError> {
    match path {
        Some(path) => {
            let json = fs::read_to_string(path)?;
            let rollup_params = serde_json::from_str::<RollupParams>(&json)
                .map_err(|err| SerdeError::new(json.to_string(), err))?;

            Ok(rollup_params)
        }
        None => Ok(default_rollup_params()),
    }
}

fn default_rollup_params() -> RollupParams {
    // TODO: load default params from a json during compile time
    RollupParams {
        rollup_name: "express".to_string(),
        block_time: 1000,
        cred_rule: block_credential::CredRule::Unchecked,
        horizon_l1_height: 3,
        genesis_l1_height: 5,
        evm_genesis_block_hash: Buf32(
            "0x37ad61cff1367467a98cf7c54c4ac99e989f1fbb1bc1e646235e90c065c565ba"
                .parse()
                .unwrap(),
        ),
        evm_genesis_block_state_root: Buf32(
            "0x351714af72d74259f45cd7eab0b04527cd40e74836a45abcae50f92d919d988f"
                .parse()
                .unwrap(),
        ),
        l1_reorg_safe_depth: 4,
        batch_l2_blocks_target: 64,
    }
}

fn main() {
    let args: Args = argh::from_env();
    if let Err(e) = main_inner(args) {
        eprintln!("FATAL ERROR: {e}");
        eprintln!("trace:\n{e:?}");
    }
}

fn main_inner(args: Args) -> anyhow::Result<()> {
    logging::init();

    // initialize the full configuration
    let config = match args.config.as_ref() {
        Some(config_path) => {
            // Values passed over arguments get the precedence over the configuration files
            let mut config = load_configuration(config_path)?;
            config.update_from_args(&args);
            config
        }
        None => match Config::from_args(&args) {
            Err(msg) => {
                eprintln!("Error: {}", msg);
                std::process::exit(1);
            }
            Ok(cfg) => cfg,
        },
    };

    // Open the database.
    let rbdb = open_rocksdb_database(&config)?;

    // Set up block params.
    let params = Params {
        rollup: load_rollup_params_or_default(&args.rollup_params).expect("rollup params"),
        run: RunParams {
            l1_follow_distance: config.sync.l1_follow_distance,
            client_checkpoint_interval: config.sync.client_checkpoint_interval,
            l2_blocks_fetch_limit: config.client.l2_blocks_fetch_limit,
        },
    };

    let params = Arc::new(params);

    // Start runtime for async IO tasks.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("express-rt")
        .build()
        .expect("init: build rt");

    // Init thread pool for batch jobs.
    // TODO switch to num_cpus maybe?  we don't want to compete with tokio though
    let pool = Arc::new(threadpool::ThreadPool::with_name(
        "express-pool".to_owned(),
        8,
    ));

    // Initialize databases.
    let l1_db = Arc::new(alpen_express_rocksdb::L1Db::new(rbdb.clone()));
    let l2_db = Arc::new(alpen_express_rocksdb::l2::db::L2Db::new(rbdb.clone()));
    let sync_ev_db = Arc::new(alpen_express_rocksdb::SyncEventDb::new(rbdb.clone()));
    let cs_db = Arc::new(alpen_express_rocksdb::ClientStateDb::new(rbdb.clone()));
    let chst_db = Arc::new(alpen_express_rocksdb::ChainStateDb::new(rbdb.clone()));
    let database = Arc::new(alpen_express_db::database::CommonDatabase::new(
        l1_db, l2_db, sync_ev_db, cs_db, chst_db,
    ));

    // Set up btcio status to pass around cheaply
    let l1_status = Arc::new(RwLock::new(L1Status::default()));

    // Set up Bitcoin client RPC.
    let bitcoind_url = format!("http://{}", config.bitcoind_rpc.rpc_url);
    let btc_rpc = alpen_express_btcio::rpc::BitcoinClient::new(
        bitcoind_url,
        config.bitcoind_rpc.rpc_user.clone(),
        config.bitcoind_rpc.rpc_password.clone(),
        bitcoin::Network::Regtest,
    );
    let btc_rpc = Arc::new(btc_rpc);

    // TODO remove this
    if config.bitcoind_rpc.network == Network::Regtest {
        warn!("network not set to regtest, ignoring");
    }

    // Init engine controller.
    let reth_jwtsecret = load_jwtsecret(&config.exec.reth.secret)?;
    let client = EngineRpcClient::from_url_secret(
        &format!("http://{}", &config.exec.reth.rpc_url),
        reth_jwtsecret,
    );

    let initial_fcs = fork_choice_state_initial(database.clone(), params.rollup())?;
    let eng_ctl = alpen_express_evmexec::engine::RpcExecEngineCtl::new(
        client,
        initial_fcs,
        rt.handle().clone(),
        database.l2_provider().clone(),
    );
    let eng_ctl = Arc::new(eng_ctl);

    // Start the sync manager.
    let sync_man = sync_manager::start_sync_tasks(
        database.clone(),
        eng_ctl.clone(),
        pool.clone(),
        params.clone(),
    )?;
    let sync_man = Arc::new(sync_man);
    let mut writer_ctl = None;

    // If the sequencer key is set, start the sequencer duties task.
    if let Some(seqkey_path) = &config.client.sequencer_key {
        info!(?seqkey_path, "initing sequencer duties task");
        let idata = load_seqkey(seqkey_path)?;

        // Set up channel and clone some things.
        let sm = sync_man.clone();
        let cu_rx = sync_man.create_cstate_subscription();
        let (duties_tx, duties_rx) = broadcast::channel::<DutyBatch>(8);
        let db = database.clone();
        let db2 = database.clone();
        let eng_ctl_de = eng_ctl.clone();
        let pool = pool.clone();

        // Spawn up writer
        let writer_config = WriterConfig::new(
            config.client.sequencer_bitcoin_address.clone(),
            config.bitcoind_rpc.network,
            params.rollup().rollup_name.clone(),
        )?;
        // Initialize SequencerDatabase
        let seqdb = Arc::new(SeqDb::new(rbdb.clone()));
        let dbseq = Arc::new(SequencerDB::new(seqdb));
        let rpc = btc_rpc.clone();
        let writer = Arc::new(start_writer_task(
            rpc,
            writer_config,
            dbseq,
            &rt,
            l1_status.clone(),
        )?);

        writer_ctl = Some(writer.clone());

        // Spawn duty tasks.
        let t_params = params.clone();
        thread::spawn(move || {
            // FIXME figure out why this can't infer the type, it's like *right there*
            duty_worker::duty_tracker_task::<_>(cu_rx, duties_tx, idata.ident, db, t_params)
        });

        let d_params = params.clone();
        thread::spawn(move || {
            duty_worker::duty_dispatch_task(
                duties_rx, idata.key, sm, db2, eng_ctl_de, writer, pool, d_params,
            )
        });
    }

    let main_fut = main_task(
        &config,
        sync_man,
        btc_rpc,
        database.clone(),
        l1_status,
        writer_ctl,
    );
    if let Err(e) = rt.block_on(main_fut) {
        error!(err = %e, "main task exited");
        process::exit(0); // special case exit once we've gotten to this point
    }

    info!("exiting");
    Ok(())
}

async fn main_task<
    D: Database + Send + Sync + 'static,
    L: L1Client,
    SD: SequencerDatabase + Send + Sync + 'static,
>(
    config: &Config,
    sync_man: Arc<SyncManager>,
    l1_rpc_client: Arc<L>,
    database: Arc<D>,
    l1_status: Arc<RwLock<L1Status>>,
    writer: Option<Arc<DaWriter<SD>>>,
) -> anyhow::Result<()>
where
    // TODO how are these not redundant trait bounds???
    <D as Database>::SeStore: Send + Sync + 'static,
    <D as Database>::L1Store: Send + Sync + 'static,
    <D as Database>::L1Prov: Send + Sync + 'static,
    <D as Database>::L2Prov: Send + Sync + 'static,
{
    // Start the L1 tasks to get that going.
    let csm_ctl = sync_man.get_csm_ctl();
    l1_reader::start_reader_tasks(
        sync_man.get_params(),
        config,
        l1_rpc_client,
        database.clone(),
        csm_ctl,
        l1_status.clone(),
    )
    .await?;

    let (stop_tx, stop_rx) = oneshot::channel();

    // Init RPC methods.
    let alp_rpc = rpc_server::AlpenRpcImpl::new(
        l1_status.clone(),
        database.clone(),
        sync_man.clone(),
        stop_tx,
    );
    let mut methods = alp_rpc.into_rpc();

    if writer.is_some() {
        let admin_rpc = rpc_server::AdminServerImpl::new(writer.unwrap().clone());
        methods.merge(admin_rpc.into_rpc())?;
    }

    let rpc_port = config.client.rpc_port;
    let rpc_server = jsonrpsee::server::ServerBuilder::new()
        .build(format!("127.0.0.1:{rpc_port}"))
        .await
        .expect("init: build rpc server");

    let rpc_handle = rpc_server.start(methods);

    // start a Btcio event handler
    info!("started RPC server");

    // Wait for a stop signal.
    let _ = stop_rx.await;

    // Now start shutdown tasks.
    if rpc_handle.stop().is_err() {
        warn!("RPC server already stopped");
    }

    Ok(())
}

fn open_rocksdb_database(
    config: &Config,
) -> anyhow::Result<Arc<rockbound::OptimisticTransactionDB>> {
    let mut database_dir = config.client.datadir.clone();
    database_dir.push("rocksdb");

    if !database_dir.exists() {
        fs::create_dir_all(&database_dir)?;
    }

    let dbname = alpen_express_rocksdb::ROCKSDB_NAME;
    let cfs = alpen_express_rocksdb::STORE_COLUMN_FAMILIES;
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let rbdb = rockbound::OptimisticTransactionDB::open(
        &database_dir,
        dbname,
        cfs.iter().map(|s| s.to_string()),
        &opts,
    )
    .context("opening database")?;

    Ok(Arc::new(rbdb))
}

fn load_seqkey(path: &PathBuf) -> anyhow::Result<IdentityData> {
    let Ok(raw_key) = <[u8; 32]>::try_from(fs::read(path)?) else {
        error!("malformed seqkey");
        anyhow::bail!("malformed seqkey");
    };

    let key = Buf32::from(raw_key);

    // FIXME all this needs to be changed to use actual cryptographic keys
    let ik = IdentityKey::Sequencer(key);
    let ident = Identity::Sequencer(key);
    let idata = IdentityData::new(ident, ik);

    Ok(idata)
}
