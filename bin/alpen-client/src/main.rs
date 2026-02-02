//! Reth node for the Alpen codebase.

#[cfg(feature = "sequencer")]
mod da_provider;
mod dummy_ol_client;
mod genesis;
mod gossip;
#[cfg(feature = "sequencer")]
mod noop_prover;
mod ol_client;
#[cfg(feature = "sequencer")]
mod payload_builder;
mod rpc_client;

#[cfg(feature = "sequencer")]
use std::fs;
use std::{env, path::PathBuf, process, sync::Arc};

use alpen_chainspec::{chain_value_parser, AlpenChainSpecParser};
#[cfg(feature = "sequencer")]
use alpen_ee_common::require_latest_batch;
use alpen_ee_common::{
    chain_status_checked, BatchStorage, BlockNumHash, ExecBlockStorage, Storage,
};
use alpen_ee_config::{AlpenEeConfig, AlpenEeParams};
use alpen_ee_database::init_db_storage;
use alpen_ee_engine::{create_engine_control_task, sync_chainstate_to_engine, AlpenRethExecEngine};
#[cfg(feature = "sequencer")]
use alpen_ee_exec_chain::{
    build_exec_chain_consensus_forwarder_task, build_exec_chain_task,
    init_exec_chain_state_from_storage,
};
#[cfg(feature = "sequencer")]
use alpen_ee_genesis::ensure_finalized_exec_chain_genesis;
use alpen_ee_genesis::{ensure_batch_genesis, ensure_genesis_ee_account_state};
use alpen_ee_ol_tracker::{init_ol_tracker_state, OLTrackerBuilder};
#[cfg(feature = "sequencer")]
use alpen_ee_sequencer::{
    block_builder_task, build_ol_chain_tracker, create_batch_builder, create_batch_lifecycle_task,
    create_update_submitter_task, init_ol_chain_tracker_state, BlockBuilderConfig,
    BlockCountDataProvider, FixedBlockCountSealing,
};
use alpen_ee_sequencer::{init_batch_builder_state, init_lifecycle_state};
use alpen_reth_node::{
    args::AlpenNodeArgs, AlpenEthereumNode, AlpenGossipProtocolHandler, AlpenGossipState,
};
#[cfg(feature = "sequencer")]
use bitcoind_async_client::{traits::Wallet as _, Auth, Client as BtcClient};
use clap::Parser;
use eyre::Context;
use reth_chainspec::ChainSpec;
use reth_cli_commands::{launcher::FnLauncher, node::NodeCommand};
use reth_cli_runner::CliRunner;
use reth_cli_util::sigsegv_handler;
use reth_network::{protocol::IntoRlpxSubProtocol, NetworkProtocols};
use reth_node_builder::{NodeBuilder, WithLaunchContext};
use reth_node_core::args::LogArgs;
use reth_provider::CanonStateSubscriptions;
use strata_acct_types::AccountId;
#[cfg(feature = "sequencer")]
use strata_btcio::{
    broadcaster::create_broadcaster_task, writer::chunked_envelope::create_chunked_envelope_task,
};
#[cfg(feature = "sequencer")]
use strata_config::btcio::WriterConfig;
use strata_identifiers::{CredRule, OLBlockId};
#[cfg(feature = "sequencer")]
use strata_params::{Params, SyncParams};
use strata_primitives::buf::Buf32;
use tokio::sync::{mpsc, watch};
use tracing::{error, info};

#[cfg(feature = "sequencer")]
use crate::{
    da_provider::{ChunkedEnvelopeDaProvider, StateDiffBlobProvider},
    noop_prover::NoopProver,
    payload_builder::AlpenRethPayloadEngine,
};
use crate::{
    dummy_ol_client::DummyOLClient,
    genesis::ee_genesis_block_info,
    gossip::{create_gossip_task, GossipConfig},
    ol_client::OLClientKind,
    rpc_client::RpcOLClient,
};

fn main() {
    sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if env::var_os("RUST_BACKTRACE").is_none() {
        // SAFETY: fine to set this in a non-async context.
        unsafe { env::set_var("RUST_BACKTRACE", "1") };
    }

    let mut command = NodeCommand::<AlpenChainSpecParser, AdditionalConfig>::parse();

    // use provided alpen chain spec
    command.chain = command.ext.custom_chain.clone();
    // enable engine api v4
    command.engine.accept_execution_requests_hash = true;
    // allow chain fork blocks to be created
    command
        .engine
        .always_process_payload_attributes_on_canonical_head = true;

    if let Err(err) = run(
        command,
        |builder: WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
         ext: AdditionalConfig| async move {
            // --- CONFIGS ---

            let datadir = builder.config().datadir().data_dir().to_path_buf();

            // TODO: read config, params from file
            let genesis_info = ee_genesis_block_info(&ext.custom_chain);

            let params = AlpenEeParams::new(
                AccountId::new([0; 32]), // TODO: correct values
                genesis_info.blockhash(),
                genesis_info.stateroot(),
                genesis_info.blocknum(),
                0,
                0,
                OLBlockId::from(Buf32([1; 32])), // TODO: correct values
            );

            info!(?params, sequencer = ext.sequencer, "Starting EE Node");

            // OL client URL is not used when dummy_ol_client is enabled
            let ol_client_url = ext.ol_client_url.clone().unwrap_or_default();

            let config = Arc::new(AlpenEeConfig::new(
                params,
                CredRule::Unchecked,
                ol_client_url,
                ext.sequencer_http,
                ext.db_retry_count,
            ));

            #[cfg(feature = "sequencer")]
            let block_builder_config = BlockBuilderConfig::default();

            // Parse sequencer private key when running in sequencer mode.
            #[cfg(feature = "sequencer")]
            let sequencer_privkey = if ext.sequencer {
                let privkey_str = env::var("SEQUENCER_PRIVATE_KEY").map_err(|_| {
                    eyre::eyre!(
                        "SEQUENCER_PRIVATE_KEY environment variable is required \
                         when running with --sequencer"
                    )
                })?;
                Some(privkey_str.parse::<Buf32>().map_err(|e| {
                    eyre::eyre!("Failed to parse SEQUENCER_PRIVATE_KEY as hex: {e}")
                })?)
            } else {
                None
            };

            let gossip_config = GossipConfig {
                sequencer_pubkey: ext.sequencer_pubkey,
                sequencer_enabled: cfg!(feature = "sequencer") && ext.sequencer,
                #[cfg(feature = "sequencer")]
                sequencer_privkey,
            };

            // --- INITIALIZE STATE ---

            let dbs = init_db_storage(&datadir, config.db_retry_count())
                .context("failed to load alpen database")?;

            let db_pool = threadpool::Builder::new()
                .num_threads(8)
                .thread_name("ee-db-pool".into())
                .build();
            let storage: Arc<_> = dbs.node_storage(db_pool.clone()).into();

            let ol_client = if ext.dummy_ol_client {
                let genesis_epoch = strata_primitives::EpochCommitment::new(
                    0,
                    config.params().genesis_ol_slot(),
                    config.params().genesis_ol_blockid(),
                );
                info!(target: "alpen-client", "Using dummy OL client (no real OL connection)");
                OLClientKind::Dummy(DummyOLClient { genesis_epoch })
            } else {
                let ol_url = ext.ol_client_url.as_ref().ok_or_else(|| {
                    eyre::eyre!("--ol-client-url is required when not using --dummy-ol-client")
                })?;
                OLClientKind::Rpc(
                    RpcOLClient::try_new(config.params().account_id(), ol_url)
                        .map_err(|e| eyre::eyre!("failed to create OL client: {e}"))?,
                )
            };
            let ol_client = Arc::new(ol_client);

            // TODO: real prover interface
            #[cfg(feature = "sequencer")]
            let batch_prover = Arc::new(NoopProver);

            ensure_genesis(config.as_ref(), storage.as_ref())
                .await
                .context("genesis should not fail")?;

            let ol_chain_status = chain_status_checked(ol_client.as_ref())
                .await
                .context("cannot fetch OL chain status")?;

            let ol_tracker_state = init_ol_tracker_state(ol_chain_status, storage.as_ref())
                .await
                .context("ol tracker state initialization should not fail")?;

            #[cfg(feature = "sequencer")]
            let ol_chain_tracker_state =
                init_ol_chain_tracker_state(storage.as_ref(), ol_client.as_ref())
                    .await
                    .context("ol chain tracker state initialization should not fail")?;

            #[cfg(feature = "sequencer")]
            let exec_chain_state = init_exec_chain_state_from_storage(storage.as_ref())
                .await
                .context("exec chain state initialization should not fail")?;

            // Non-sequencer fallback: use OL tracker head with block number 0.
            // Will be updated by gossip once peers are connected.
            let initial_preconf_head = {
                let hash = ol_tracker_state.best_ee_state().last_exec_blkid();
                BlockNumHash::new(hash, 0)
            };

            // In sequencer mode, override with the actual exec chain tip.
            #[cfg(feature = "sequencer")]
            let initial_preconf_head = if ext.sequencer {
                exec_chain_state.tip_blocknumhash()
            } else {
                initial_preconf_head
            };

            let batch_builder_state = init_batch_builder_state(storage.as_ref())
                .await
                .context("batch builder state initialization should not fail")?;

            let batch_lifecycle_state = init_lifecycle_state(storage.as_ref())
                .await
                .context("batch lifecycle state initialization should not fail")?;
            // --- INITIALIZE SERVICES ---

            // Create gossip channel before building the node so we can register it early
            let (gossip_tx, gossip_rx) = mpsc::unbounded_channel();

            // Create preconf channel for p2p head block gossip -> engine control integration
            // This channel sends block hash and number received from peers to the engine control
            // task
            let (preconf_tx, preconf_rx) = watch::channel(initial_preconf_head);

            let (ol_tracker, ol_tracker_task) = OLTrackerBuilder::new(
                ol_tracker_state,
                config.params().clone(),
                storage.clone(),
                ol_client.clone(),
            )
            .build();

            let mut node_builder = builder
                .node(AlpenEthereumNode::new(AlpenNodeArgs::default()))
                // Register Alpen gossip RLPx subprotocol
                .on_component_initialized({
                    let gossip_tx = gossip_tx.clone();
                    move |node| {
                        // Add the custom RLPx subprotocol before node fully starts
                        // See: crates/reth/node/src/gossip/
                        let handler =
                            AlpenGossipProtocolHandler::new(AlpenGossipState::new(gossip_tx));
                        node.components
                            .network
                            .add_rlpx_sub_protocol(handler.into_rlpx_sub_protocol());
                        info!(target: "alpen-gossip", "Registered Alpen gossip RLPx subprotocol");
                        Ok(())
                    }
                });

            // Install state diff exex for sequencer DA.
            // The exex persists per-block state diffs that the blob provider reads.
            if ext.sequencer {
                node_builder = node_builder.install_exex("state_diffs", {
                    let state_diff_db = dbs.witness_db.clone();
                    |ctx| async {
                        Ok(alpen_reth_exex::StateDiffGenerator::new(ctx, state_diff_db).start())
                    }
                });
                info!(target: "alpen-client", "installed StateDiffGenerator exex for DA");
            }

            let handle = node_builder.launch().await?;

            let node = handle.node;

            // Sync chainstate to engine for sequencer nodes before starting other tasks
            #[cfg(feature = "sequencer")]
            if ext.sequencer {
                let engine = AlpenRethExecEngine::new(node.beacon_engine_handle.clone());
                sync_chainstate_to_engine(storage.as_ref(), &node.provider, &engine)
                    .await
                    .context("failed to sync chainstate to engine on startup")?;
                info!(target: "alpen-client", "chainstate sync completed successfully");
            }

            let engine_control_task = create_engine_control_task(
                preconf_rx.clone(),
                ol_tracker.consensus_watcher(),
                node.provider.clone(),
                AlpenRethExecEngine::new(node.beacon_engine_handle.clone()),
            );

            // Subscribe to canonical state notifications for broadcasting new blocks
            let state_events = node.provider.subscribe_to_canonical_state();

            // Create gossip task for broadcasting new blocks
            let gossip_task =
                create_gossip_task(gossip_rx, state_events, preconf_tx.clone(), gossip_config);

            // Spawn critical tasks
            node.task_executor
                .spawn_critical("ol_tracker_task", ol_tracker_task);
            node.task_executor
                .spawn_critical("engine_control", engine_control_task);
            node.task_executor
                .spawn_critical("gossip_task", gossip_task);

            #[cfg(feature = "sequencer")]
            if ext.sequencer {
                let payload_engine = Arc::new(AlpenRethPayloadEngine::new(
                    node.payload_builder_handle.clone(),
                    node.beacon_engine_handle.clone(),
                ));

                let (exec_chain_handle, exec_chain_task) =
                    build_exec_chain_task(exec_chain_state, preconf_tx.clone(), storage.clone());

                let (ol_chain_tracker, ol_chain_tracker_task) = build_ol_chain_tracker(
                    ol_chain_tracker_state,
                    ol_tracker.ol_status_watcher(),
                    ol_client.clone(),
                    storage.clone(),
                );

                let (latest_batch, _) = require_latest_batch(storage.as_ref()).await?;

                let batch_sealing_policy = FixedBlockCountSealing::new(100);
                let block_data_provider = Arc::new(BlockCountDataProvider);

                let (batch_builder_handle, batch_builder_task) = create_batch_builder(
                    latest_batch.id(),
                    BlockNumHash::new(genesis_info.blockhash().0.into(), genesis_info.blocknum()),
                    batch_builder_state,
                    preconf_rx,
                    block_data_provider,
                    batch_sealing_policy,
                    storage.clone(),
                    storage.clone(),
                    exec_chain_handle.clone(),
                );

                // DA pipeline 
                //
                // clap `requires_all` on --sequencer guarantees all DA args are present.
                let magic_bytes = ext.magic_bytes.expect("enforced by clap");
                let btc_url = ext.btc_rpc_url.as_ref().expect("enforced by clap");
                let btc_user = ext.btc_rpc_user.as_ref().expect("enforced by clap");
                let btc_pass = ext.btc_rpc_password.as_ref().expect("enforced by clap");
                let params_path = ext.rollup_params.as_ref().expect("enforced by clap");

                // Load rollup params from the JSON file.
                let rollup_json =
                    fs::read_to_string(params_path).context("reading rollup params file")?;
                let rollup_params: strata_params::RollupParams =
                    serde_json::from_str(&rollup_json).context("parsing rollup params JSON")?;

                // TODO: decouple btcio from Params struct
                let params = Arc::new(Params {
                    rollup: rollup_params,
                    // SyncParams are unused by the broadcaster/watcher; zero-fill.
                    run: SyncParams {
                        l1_follow_distance: 0,
                        client_checkpoint_interval: 0,
                        l2_blocks_fetch_limit: 0,
                    },
                });

                // Bitcoin RPC client.
                let btc_client = Arc::new(
                    BtcClient::new(
                        btc_url.clone(),
                        Auth::UserPass(btc_user.clone(), btc_pass.clone()),
                        None,
                        None,
                        None,
                    )
                    .map_err(|e| eyre::eyre!("creating Bitcoin RPC client: {e}"))?,
                );

                // Sequencer address from bitcoin wallet.
                let sequencer_address = btc_client
                    .get_new_address()
                    .await
                    .map_err(|e| eyre::eyre!("failed to get sequencer address: {e}"))?;

                // Wrap raw DBs in ops using the shared DB threadpool.
                let broadcast_ops = Arc::new(dbs.broadcast_ops(db_pool.clone()));
                let envelope_ops = Arc::new(dbs.chunked_envelope_ops(db_pool.clone()));

                // TODO: make broadcast_poll_interval configurable via CLI
                let broadcast_poll_interval = 5_000;
                let (broadcast_handle, broadcaster_task) = create_broadcaster_task(
                    btc_client.clone(),
                    broadcast_ops.clone(),
                    params.clone(),
                    broadcast_poll_interval,
                );

                let writer_config = Arc::new(WriterConfig::default());
                let (envelope_handle, envelope_watcher_task) = create_chunked_envelope_task(
                    btc_client,
                    writer_config,
                    params,
                    sequencer_address,
                    envelope_ops,
                    broadcast_handle.clone(),
                )
                .map_err(|e| eyre::eyre!("creating chunked envelope task: {e}"))?;

                let blob_provider: Arc<dyn alpen_ee_common::DaBlobProvider> = Arc::new(
                    StateDiffBlobProvider::new(storage.clone(), dbs.witness_db.clone()),
                );

                let batch_da_provider = Arc::new(ChunkedEnvelopeDaProvider::new(
                    blob_provider,
                    envelope_handle,
                    broadcast_ops,
                    magic_bytes,
                ));

                // Spawn btcio tasks.
                node.task_executor
                    .spawn_critical("l1_broadcaster", broadcaster_task);
                node.task_executor
                    .spawn_critical("chunked_envelope_watcher", envelope_watcher_task);

                info!(target: "alpen-client", "btcio DA pipeline started");

                let (batch_lifecycle_handle, batch_lifecycle_task) = create_batch_lifecycle_task(
                    None,
                    batch_lifecycle_state,
                    batch_builder_handle.latest_batch_watcher(),
                    batch_da_provider,
                    batch_prover.clone(),
                    storage.clone(),
                );

                let update_submitter_task = create_update_submitter_task(
                    ol_client,
                    storage.clone(),
                    storage.clone(),
                    batch_prover,
                    batch_lifecycle_handle.latest_proof_ready_watcher(),
                    ol_tracker.ol_status_watcher(),
                );

                node.task_executor
                    .spawn_critical("exec_chain", exec_chain_task);
                node.task_executor.spawn_critical(
                    "exec_chain_consensus_forwarder",
                    build_exec_chain_consensus_forwarder_task(
                        exec_chain_handle.clone(),
                        ol_tracker.consensus_watcher(),
                    ),
                );
                node.task_executor
                    .spawn_critical("ol_chain_tracker", ol_chain_tracker_task);
                node.task_executor.spawn_critical(
                    "block_assembly",
                    block_builder_task(
                        block_builder_config,
                        exec_chain_handle,
                        ol_chain_tracker,
                        payload_engine,
                        storage.clone(),
                    ),
                );

                node.task_executor
                    .spawn_critical("ee_batch_builder", batch_builder_task);
                node.task_executor
                    .spawn_critical("ee_batch_lifecycle", batch_lifecycle_task);
                node.task_executor
                    .spawn_critical("ee_update_submitter", update_submitter_task);
                // TODO: proof generation
                // TODO: post update to OL
            }

            handle.node_exit_future.await
        },
    ) {
        eprintln!("Error: {err:?}");
        process::exit(1);
    }
}

/// Our custom cli args extension that adds one flag to reth default CLI.
#[derive(Debug, clap::Parser)]
pub struct AdditionalConfig {
    #[command(flatten)]
    pub logs: LogArgs,

    /// The chain this node is running.
    ///
    /// Possible values are either a built-in chain or the path to a chain specification file.
    /// Cannot override existing `chain` arg, so this is a workaround.
    #[arg(
        long,
        value_name = "CHAIN_OR_PATH",
        default_value = "testnet",
        value_parser = chain_value_parser,
        required = false,
    )]
    pub custom_chain: Arc<ChainSpec>,

    /// Rpc of sequencer's reth node to forward transactions to.
    #[arg(long, required = false)]
    pub sequencer_http: Option<String>,

    /// URL of OL node RPC (can be either `http[s]://` or `ws[s]://`).
    /// Required unless `--dummy-ol-client` is specified.
    #[arg(long)]
    pub ol_client_url: Option<String>,

    /// Use a dummy OL client instead of connecting to a real OL node.
    /// This is useful for testing EE functionality in isolation.
    ///
    /// NOTE: This is intentionally separate from OL-EE integration tests which
    /// need the real OL RPC client. The dummy client is only for EE-specific
    /// tests that don't need OL interaction.
    #[arg(long, default_value_t = false)]
    pub dummy_ol_client: bool,

    #[arg(long, required = false)]
    pub db_retry_count: Option<u16>,

    /// Run the node as a sequencer. Requires the `sequencer` feature,
    /// a `SEQUENCER_PRIVATE_KEY` environment variable, and all DA-related
    /// arguments (`--magic-bytes`, `--btc-rpc-url`, `--btc-rpc-user`,
    /// `--btc-rpc-password`, `--rollup-params`).
    #[arg(
        long,
        default_value_t = false,
        requires_all = ["magic_bytes", "btc_rpc_url", "btc_rpc_user", "btc_rpc_password", "rollup_params"],
    )]
    pub sequencer: bool,

    /// Sequencer's public key (hex-encoded, 32 bytes) for signature validation.
    #[arg(long, required = true, value_parser = parse_buf32)]
    pub sequencer_pubkey: Buf32,

    /// Magic bytes (hex-encoded, 4 bytes) for tagging DA envelope transactions.
    ///
    /// Must match the rollup's `RollupParams.magic_bytes`. Example: `deadbeef`.
    #[arg(long, required = false, value_parser = parse_magic_bytes)]
    pub magic_bytes: Option<[u8; 4]>,

    /// Bitcoin Core RPC URL. Required when `--sequencer` is set.
    #[arg(long, required = false)]
    pub btc_rpc_url: Option<String>,

    /// Bitcoin Core RPC username. Required when `--sequencer` is set.
    #[arg(long, required = false)]
    pub btc_rpc_user: Option<String>,

    /// Bitcoin Core RPC password. Required when `--sequencer` is set.
    #[arg(long, required = false)]
    pub btc_rpc_password: Option<String>,

    /// Path to rollup params JSON file. Required when `--sequencer` is set.
    ///
    /// The broadcaster uses `l1_reorg_safe_depth` from these params to determine
    /// when a transaction is finalized.
    #[arg(long, required = false)]
    pub rollup_params: Option<PathBuf>,
}

/// Run node with logging
/// based on reth::cli::Cli::run
fn run<L>(
    mut command: NodeCommand<AlpenChainSpecParser, AdditionalConfig>,
    launcher: L,
) -> eyre::Result<()>
where
    L: std::ops::AsyncFnOnce(
        WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
        AdditionalConfig,
    ) -> eyre::Result<()>,
{
    command.ext.logs.log_file_directory = command
        .ext
        .logs
        .log_file_directory
        .join(command.chain.chain.to_string());

    let _guard = command.ext.logs.init_tracing()?;
    info!(target: "reth::cli", cmd = %command.ext.logs.log_file_directory, "Initialized tracing, debug log directory");

    if command.ext.sequencer && !cfg!(feature = "sequencer") {
        error!(
            target: "alpen-client",
            "Sequencer flag enabled but binary built without `sequencer` feature. Rebuild with default features or enable the `sequencer` feature."
        );
        eyre::bail!("sequencer feature not enabled at compile time");
    }

    let runner = CliRunner::try_default_runtime()?;
    runner.run_command_until_exit(|ctx| {
        command.execute(
            ctx,
            FnLauncher::new::<AlpenChainSpecParser, AdditionalConfig>(launcher),
        )
    })?;

    Ok(())
}

/// Parse a hex-encoded string into a [`Buf32`].
fn parse_buf32(s: &str) -> eyre::Result<Buf32> {
    s.parse::<Buf32>()
        .map_err(|e| eyre::eyre!("Failed to parse hex string as Buf32: {e}"))
}

/// Parse a hex-encoded 4-byte magic bytes string (e.g. `"deadbeef"`).
fn parse_magic_bytes(s: &str) -> eyre::Result<[u8; 4]> {
    let bytes = hex::decode(s).map_err(|e| eyre::eyre!("invalid hex: {e}"))?;
    <[u8; 4]>::try_from(bytes.as_slice())
        .map_err(|_| eyre::eyre!("expected exactly 4 bytes, got {}", bytes.len()))
}

/// Handle genesis related tasks.
/// Mainly deals with ensuring database has minimal expected state.
async fn ensure_genesis<TStorage: Storage + ExecBlockStorage + BatchStorage>(
    config: &AlpenEeConfig,
    storage: &TStorage,
) -> eyre::Result<()> {
    ensure_genesis_ee_account_state(config, storage).await?;
    #[cfg(feature = "sequencer")]
    ensure_finalized_exec_chain_genesis(config, storage).await?;
    #[cfg(feature = "sequencer")]
    ensure_batch_genesis(config, storage).await?;
    Ok(())
}
