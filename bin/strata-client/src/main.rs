use std::{str::FromStr, sync::Arc, time::Duration};

use bitcoin::{hashes::Hash, Address, BlockHash};
use config::{ClientMode, Config, SequencerConfig};
use el_sync::sync_chainstate_to_el;
use jsonrpsee::Methods;
use rpc_client::sync_client;
use strata_bridge_relay::relayer::RelayerHandle;
use strata_btcio::{
    broadcaster::{spawn_broadcaster_task, L1BroadcastHandle},
    rpc::{traits::Reader, BitcoinClient},
    writer::{config::WriterConfig, start_inscription_task},
};
use strata_common::{env::parse_env_or, logging};
use strata_consensus_logic::{
    checkpoint::CheckpointHandle,
    duty::{types::DutyBatch, worker as duty_worker},
    genesis,
    sync_manager::{self, SyncManager},
};
use strata_db::{
    traits::{ChainstateProvider, Database, L2DataProvider, L2DataStore},
    DbError,
};
use strata_eectl::engine::ExecEngineCtl;
use strata_evmexec::{engine::RpcExecEngineCtl, EngineRpcClient};
use strata_primitives::params::{Params, SyncParams};
use strata_rocksdb::{
    broadcaster::db::BroadcastDatabase, sequencer::db::SequencerDB, DbOpsConfig, RBSeqBlobDb,
};
use strata_rpc_api::{StrataAdminApiServer, StrataApiServer, StrataSequencerApiServer};
use strata_status::{StatusRx, StatusTx};
use strata_storage::{
    managers::checkpoint::CheckpointDbManager, ops::bridge_relay::BridgeMsgOps, L2BlockManager,
};
use strata_sync::{self, L2SyncContext, RpcSyncPeer};
use strata_tasks::{ShutdownSignal, TaskExecutor, TaskManager};
use tokio::{
    runtime::{Handle, Runtime},
    sync::{broadcast, oneshot},
};
use tracing::*;

use crate::{args::Args, helpers::*};

mod args;
mod config;
mod el_sync;
mod errors;
mod extractor;
mod helpers;
mod l1_reader;
mod network;
mod rpc_client;
mod rpc_server;

// TODO: this might need to come from config.
const BITCOIN_POLL_INTERVAL: u64 = 200; // millis
const SEQ_ADDR_GENERATION_TIMEOUT: u64 = 10; // seconds
const SYNC_BATCH_SIZE_ENVVAR: &str = "SYNC_BATCH_SIZE";

fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();
    if let Err(e) = main_inner(args) {
        eprintln!("FATAL ERROR: {e}");
        // eprintln!("trace:\n{e:?}");
        // TODO: error code ?

        return Err(e);
    }

    Ok(())
}

fn main_inner(args: Args) -> anyhow::Result<()> {
    // Start runtime for async IO tasks.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("strata-rt")
        .build()
        .expect("init: build rt");

    // Init the logging before we do anything else.
    init_logging(runtime.handle());

    let config = get_config(args.clone())?;

    // Set up block params.
    let rparams = resolve_and_validate_rollup_params(args.rollup_params.as_deref())
        .map_err(anyhow::Error::from)?;
    let params: Arc<_> = Params {
        rollup: rparams,
        run: SyncParams {
            // FIXME these shouldn't be configurable here
            l1_follow_distance: config.sync.l1_follow_distance,
            client_checkpoint_interval: config.sync.client_checkpoint_interval,
            l2_blocks_fetch_limit: config.client.l2_blocks_fetch_limit,
        },
    }
    .into();

    let mut methods = jsonrpsee::Methods::new();

    // Open and initialize the database.
    let rbdb = open_rocksdb_database(&config)?;
    let ops_config = DbOpsConfig::new(config.client.db_retry_count);

    // initialize core databases
    let database = init_core_dbs(rbdb.clone(), ops_config);

    // Init thread pool for batch jobs.
    // TODO switch to num_cpus
    let pool = threadpool::ThreadPool::with_name("strata-pool".to_owned(), 8);

    let task_manager = TaskManager::new(runtime.handle().clone());
    let executor = task_manager.executor();

    // Set up bridge messaging stuff.
    // TODO move all of this into relayer task init
    let bridge_msg_db = Arc::new(strata_rocksdb::BridgeMsgDb::new(rbdb.clone(), ops_config));
    let bridge_msg_ctx = strata_storage::ops::bridge_relay::Context::new(bridge_msg_db);
    let bridge_msg_ops = Arc::new(bridge_msg_ctx.into_ops(pool.clone()));

    let checkpoint_manager: Arc<_> =
        CheckpointDbManager::new(pool.clone(), database.clone()).into();
    let checkpoint_handle: Arc<_> = CheckpointHandle::new(checkpoint_manager.clone()).into();
    let bitcoin_client = create_bitcoin_rpc_client(&config)?;

    let l2_block_manager = Arc::new(L2BlockManager::new(pool.clone(), database.clone()));

    // Check if we have to do genesis.
    if genesis::check_needs_client_init(database.as_ref())? {
        info!("need to init client state!");
        genesis::init_client_state(&params, database.as_ref())?;
    }

    info!("init finished, starting main tasks");

    let ctx = start_core_tasks(
        &executor,
        pool,
        &runtime,
        &config,
        params.clone(),
        database,
        l2_block_manager,
        checkpoint_manager,
        bridge_msg_ops,
        bitcoin_client,
    )?;

    match &config.client.client_mode {
        // If we're a sequencer, start the sequencer db and duties task.
        ClientMode::Sequencer(sequencer_config) => {
            let broadcast_database = init_broadcaster_database(rbdb.clone(), ops_config);
            let broadcast_handle = start_broadcaster_tasks(
                broadcast_database,
                ctx.pool.clone(),
                &executor,
                ctx.bitcoin_client.clone(),
                params.clone(),
            );
            let seq_db = init_sequencer_database(rbdb.clone(), ops_config);

            start_sequencer_tasks(
                ctx.clone(),
                &config,
                sequencer_config,
                &executor,
                &runtime,
                seq_db,
                checkpoint_handle.clone(),
                broadcast_handle,
                &mut methods,
            )?;
        }
        ClientMode::FullNode(fullnode_config) => {
            let sequencer_rpc = &fullnode_config.sequencer_rpc;
            info!(?sequencer_rpc, "initing fullnode task");

            let rpc_client = sync_client(sequencer_rpc);
            let download_batch_size = parse_env_or(SYNC_BATCH_SIZE_ENVVAR, 10);
            let sync_peer = RpcSyncPeer::new(rpc_client, download_batch_size);
            let l2_sync_context = L2SyncContext::new(
                sync_peer,
                ctx.l2_block_manager.clone(),
                ctx.sync_manager.clone(),
            );
            // NOTE: this might block for some time during first run with empty db until genesis
            // block is generated
            let mut l2_sync_state =
                strata_sync::block_until_csm_ready_and_init_sync_state(&l2_sync_context)?;

            executor.spawn_critical_async("l2-sync-manager", async move {
                strata_sync::sync_worker(&mut l2_sync_state, &l2_sync_context)
                    .await
                    .map_err(Into::into)
            });
        }
    }

    executor.spawn_critical_async(
        "main-rpc",
        start_rpc(
            ctx,
            task_manager.shutdown_signal(),
            config,
            args,
            checkpoint_handle,
            methods,
        ),
    );

    task_manager.start_signal_listeners();
    task_manager.monitor(Some(Duration::from_secs(5)))?;

    info!("exiting");
    Ok(())
}

/// Sets up the logging system given a handle to a runtime context to possibly
/// start the OTLP output on.
fn init_logging(rt: &Handle) {
    let mut lconfig = logging::LoggerConfig::with_base_name("strata-client");

    // Set the OpenTelemetry URL if set.
    let otlp_url = logging::get_otlp_url_from_env();
    if let Some(url) = &otlp_url {
        lconfig.set_otlp_url(url.clone());
    }

    {
        // Need to set the runtime context because of nonsense.
        let _g = rt.enter();
        logging::init(lconfig);
    }

    // Have to log this after we start the logging formally.
    if let Some(url) = &otlp_url {
        info!(%url, "using OpenTelemetry tracing output");
    }
}

#[derive(Clone)]
pub struct CoreContext {
    pub database: Arc<CommonDb>,
    pub pool: threadpool::ThreadPool,
    pub params: Arc<Params>,
    pub sync_manager: Arc<SyncManager>,
    pub l2_block_manager: Arc<L2BlockManager>,
    pub status_tx: Arc<StatusTx>,
    pub status_rx: Arc<StatusRx>,
    pub engine: Arc<RpcExecEngineCtl<EngineRpcClient>>,
    pub relayer_handle: Arc<RelayerHandle>,
    pub bitcoin_client: Arc<BitcoinClient>,
}

fn do_startup_checks(
    database: &impl Database,
    engine: &impl ExecEngineCtl,
    bitcoin_client: &impl Reader,
    runtime: &Runtime,
) -> anyhow::Result<()> {
    let chain_state_prov = database.chain_state_provider();
    let last_state_idx = match chain_state_prov.get_last_state_idx() {
        Ok(idx) => idx,
        Err(DbError::NotBootstrapped) => {
            // genesis is not done
            info!("startup: awaiting genesis");
            return Ok(());
        }
        err => err?,
    };
    let Some(last_chain_state) = chain_state_prov.get_toplevel_state(last_state_idx)? else {
        anyhow::bail!(format!("Missing chain state idx: {}", last_state_idx));
    };

    // Check that we can connect to bitcoin client and block we believe to be matured in L1 is
    // actually present
    let safe_l1blockid = last_chain_state.l1_view().safe_block().blkid();
    let block_hash = BlockHash::from_slice(safe_l1blockid.as_ref())?;

    match runtime.block_on(bitcoin_client.get_block(&block_hash)) {
        Ok(_block) => {
            info!("startup: last matured block: {}", block_hash);
        }
        Err(client_error) if client_error.is_block_not_found() => {
            anyhow::bail!("Missing expected block: {}", block_hash);
        }
        Err(client_error) => {
            anyhow::bail!("could not connect to bitcoin, err = {}", client_error);
        }
    }

    // Check that tip L2 block exists (and engine can be connected to)
    let chain_tip = last_chain_state.chain_tip_blockid();
    match engine.check_block_exists(chain_tip) {
        Ok(true) => {
            info!("startup: last l2 block is synced")
        }
        Ok(false) => {
            // Current chain tip tip block is not known by the EL.
            warn!("missing expected evm block, block_id = {}", chain_tip);
            sync_chainstate_to_el(database, engine)?;
        }
        Err(error) => {
            // Likely network issue
            anyhow::bail!("could not connect to exec engine, err = {}", error);
        }
    }

    // remove any extra L2 blocks beyond latest state idx
    // this will resolve block production issue due to block already being in L2 db
    let l2_prov = database.l2_provider();
    let l2_store = database.l2_store();
    let mut extra_blockids = Vec::new();
    let mut blockidx = last_state_idx + 1;
    loop {
        info!(?blockidx, "check for extra blocks beyond latest state idx");
        let mut blockids = l2_prov.get_blocks_at_height(blockidx)?;
        if blockids.is_empty() {
            break;
        }
        info!(?blockidx, ?blockids, "found extra blocks");
        extra_blockids.append(&mut blockids);
        blockidx += 1;
    }
    info!(count = extra_blockids.len(), "total extra blocks found");
    for blockid in extra_blockids {
        info!(?blockid, "removing extra block");
        l2_store.del_block_data(blockid)?;
    }

    // everything looks ok
    info!("Startup checks passed");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn start_core_tasks(
    executor: &TaskExecutor,
    pool: threadpool::ThreadPool,
    runtime: &Runtime,
    config: &Config,
    params: Arc<Params>,
    database: Arc<CommonDb>,
    l2_block_manager: Arc<L2BlockManager>,
    checkpoint_manager: Arc<CheckpointDbManager>,
    bridge_msg_ops: Arc<BridgeMsgOps>,
    bitcoin_client: Arc<BitcoinClient>,
) -> anyhow::Result<CoreContext> {
    // init status tasks
    let (status_tx, status_rx) = init_status_channel(database.as_ref(), params.network())?;

    let engine = init_engine_controller(
        config,
        database.clone(),
        params.as_ref(),
        l2_block_manager.clone(),
        runtime,
    )?;

    // do startup checks
    do_startup_checks(
        database.as_ref(),
        engine.as_ref(),
        bitcoin_client.as_ref(),
        runtime,
    )?;

    // Start the sync manager.
    let sync_manager: Arc<_> = sync_manager::start_sync_tasks(
        executor,
        database.clone(),
        l2_block_manager.clone(),
        engine.clone(),
        pool.clone(),
        params.clone(),
        (status_tx.clone(), status_rx.clone()),
        checkpoint_manager,
    )?
    .into();

    // Start the L1 tasks to get that going.
    l1_reader::start_reader_tasks(
        executor,
        sync_manager.get_params(),
        config,
        bitcoin_client.clone(),
        database.clone(),
        sync_manager.get_csm_ctl(),
        status_tx.clone(),
    )?;

    // Start relayer task.
    let relayer_handle = strata_bridge_relay::relayer::start_bridge_relayer_task(
        bridge_msg_ops,
        status_rx.clone(),
        config.relayer,
        executor,
    );

    Ok(CoreContext {
        database,
        pool,
        params,
        sync_manager,
        l2_block_manager,
        status_tx,
        status_rx,
        engine,
        relayer_handle,
        bitcoin_client,
    })
}

#[allow(clippy::too_many_arguments)]
fn start_sequencer_tasks(
    ctx: CoreContext,
    config: &Config,
    sequencer_config: &SequencerConfig,
    executor: &TaskExecutor,
    runtime: &Runtime,
    seq_db: Arc<SequencerDB<RBSeqBlobDb>>,
    checkpoint_handle: Arc<CheckpointHandle>,
    broadcast_handle: Arc<L1BroadcastHandle>,
    methods: &mut Methods,
) -> anyhow::Result<()> {
    let CoreContext {
        database,
        pool,
        params,
        sync_manager,
        l2_block_manager,
        status_tx,
        engine,
        bitcoin_client,
        ..
    } = ctx;

    info!(seqkey_path = ?sequencer_config.sequencer_key, "initing sequencer duties task");
    let idata = load_seqkey(&sequencer_config.sequencer_key)?;

    // Set up channel and clone some things.
    let (duties_tx, duties_rx) = broadcast::channel::<DutyBatch>(8);

    // Use provided address or generate an address owned by the sequencer's bitcoin wallet
    let sequencer_bitcoin_address = match sequencer_config.sequencer_bitcoin_address.as_ref() {
        Some(address) => {
            Address::from_str(address)?.require_network(config.bitcoind_rpc.network)?
        }
        None => runtime.block_on(generate_sequencer_address(
            &bitcoin_client,
            SEQ_ADDR_GENERATION_TIMEOUT,
            BITCOIN_POLL_INTERVAL,
        ))?,
    };

    // Spawn up writer
    let writer_config = WriterConfig::new(
        sequencer_bitcoin_address,
        params.rollup().rollup_name.clone(),
    )?;

    // Start inscription tasks
    let inscription_handle = start_inscription_task(
        executor,
        bitcoin_client,
        writer_config,
        seq_db,
        status_tx.clone(),
        pool.clone(),
        broadcast_handle.clone(),
    )?;

    let admin_rpc = rpc_server::SequencerServerImpl::new(
        inscription_handle.clone(),
        broadcast_handle,
        params.clone(),
        checkpoint_handle.clone(),
    );
    methods.merge(admin_rpc.into_rpc())?;

    // Spawn duty tasks.
    let cupdate_rx = sync_manager.create_cstate_subscription();
    let t_l2_block_manager = l2_block_manager.clone();
    let t_params = params.clone();
    let t_database = database.clone();
    executor.spawn_critical("duty_worker::duty_tracker_task", move |shutdown| {
        duty_worker::duty_tracker_task(
            shutdown,
            cupdate_rx,
            duties_tx,
            idata.ident,
            t_database,
            t_l2_block_manager,
            t_params,
        )
        .map_err(Into::into)
    });

    let d_executor = executor.clone();
    executor.spawn_critical("duty_worker::duty_dispatch_task", move |shutdown| {
        duty_worker::duty_dispatch_task(
            shutdown,
            d_executor,
            duties_rx,
            idata.key,
            sync_manager,
            database,
            engine,
            inscription_handle,
            pool,
            params,
            checkpoint_handle,
        )
    });

    Ok(())
}

fn start_broadcaster_tasks(
    broadcast_database: Arc<BroadcastDatabase>,
    pool: threadpool::ThreadPool,
    executor: &TaskExecutor,
    bitcoin_client: Arc<BitcoinClient>,
    params: Arc<Params>,
) -> Arc<L1BroadcastHandle> {
    // Set up L1 broadcaster.
    let broadcast_ctx = strata_storage::ops::l1tx_broadcast::Context::new(broadcast_database);
    let broadcast_ops = Arc::new(broadcast_ctx.into_ops(pool));
    // start broadcast task
    let broadcast_handle =
        spawn_broadcaster_task(executor, bitcoin_client.clone(), broadcast_ops, params);
    Arc::new(broadcast_handle)
}

async fn start_rpc(
    ctx: CoreContext,
    shutdown_signal: ShutdownSignal,
    config: Config,
    args: Args,
    checkpoint_handle: Arc<CheckpointHandle>,
    mut methods: Methods,
) -> anyhow::Result<()> {
    let CoreContext {
        database,
        sync_manager,
        l2_block_manager,
        status_rx,
        relayer_handle,
        ..
    } = ctx;

    let (stop_tx, stop_rx) = oneshot::channel();

    // Init RPC impls.
    let strata_rpc = rpc_server::StrataRpcImpl::new(
        status_rx,
        database,
        sync_manager,
        l2_block_manager,
        checkpoint_handle,
        relayer_handle,
    );
    methods.merge(strata_rpc.into_rpc())?;

    if args.enable_admin_rpc {
        let admin_rpc = rpc_server::AdminServerImpl::new(stop_tx);
        methods.merge(admin_rpc.into_rpc())?;
    }

    let rpc_host = config.client.rpc_host;
    let rpc_port = config.client.rpc_port;

    let rpc_server = jsonrpsee::server::ServerBuilder::new()
        .build(format!("{rpc_host}:{rpc_port}"))
        .await
        .expect("init: build rpc server");

    let rpc_handle = rpc_server.start(methods);

    // start a Btcio event handler
    info!("started RPC server");

    // Wait for a stop signal.
    let _ = stop_rx.await;

    // Send shutdown to all tasks
    shutdown_signal.send();

    // Now start shutdown tasks.
    if rpc_handle.stop().is_err() {
        warn!("RPC server already stopped");
    }

    // wait for rpc to stop
    rpc_handle.stopped().await;

    Ok(())
}
