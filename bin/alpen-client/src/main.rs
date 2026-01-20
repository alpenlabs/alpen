//! Reth node for the Alpen codebase.

mod dummy_ol_client;
mod genesis;
mod gossip;
#[cfg(feature = "sequencer")]
mod payload_builder;
mod rpc_client;

use std::{env, process, sync::Arc};

use alpen_chainspec::{chain_value_parser, AlpenChainSpecParser};
use alpen_ee_common::{chain_status_checked, ExecBlockStorage, Storage};
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
use alpen_ee_genesis::ensure_genesis_ee_account_state;
use alpen_ee_ol_tracker::{init_ol_tracker_state, OLTrackerBuilder};
#[cfg(feature = "sequencer")]
use alpen_ee_sequencer::{
    block_builder_task, build_ol_chain_tracker, init_ol_chain_tracker_state, BlockBuilderConfig,
};
use alpen_reth_node::{
    args::AlpenNodeArgs, AlpenEthereumNode, AlpenGossipProtocolHandler, AlpenGossipState,
};
use clap::Parser;
use dummy_ol_client::DummyOLClient;
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
use strata_identifiers::{CredRule, OLBlockId};
use strata_primitives::buf::Buf32;
use tokio::sync::{mpsc, watch};
use tracing::{error, info};

#[cfg(feature = "sequencer")]
use crate::payload_builder::AlpenRethPayloadEngine;
use crate::{
    genesis::ee_genesis_block_info,
    gossip::{create_gossip_task, GossipConfig},
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
    // disable peer discovery
    command.network.discovery.disable_discovery = true;
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

            let config = Arc::new(AlpenEeConfig::new(
                params,
                CredRule::Unchecked,
                ext.ol_client_http,
                ext.sequencer_http,
                ext.db_retry_count,
            ));

            #[cfg(feature = "sequencer")]
            let block_builder_config = BlockBuilderConfig::default();

            // Parse sequencer private key from environment variable (only in sequencer mode)
            let gossip_config = {
                #[cfg(feature = "sequencer")]
                {
                    let sequencer_privkey = if ext.sequencer {
                        let privkey_str = env::var("SEQUENCER_PRIVATE_KEY").map_err(|_| {
                            eyre::eyre!("SEQUENCER_PRIVATE_KEY environment variable is required when running with --sequencer")
                        })?;
                        Some(privkey_str.parse::<Buf32>().map_err(|e| {
                            eyre::eyre!("Failed to parse SEQUENCER_PRIVATE_KEY as hex: {e}")
                        })?)
                    } else {
                        None
                    };

                    GossipConfig {
                        sequencer_pubkey: ext.sequencer_pubkey,
                        sequencer_enabled: ext.sequencer,
                        sequencer_privkey,
                    }
                }

                #[cfg(not(feature = "sequencer"))]
                {
                    GossipConfig {
                        sequencer_pubkey: ext.sequencer_pubkey,
                        sequencer_enabled: false,
                    }
                }
            };

            // --- INITIALIZE STATE ---

            let storage: Arc<_> = init_db_storage(&datadir, config.db_retry_count())
                .context("failed to load alpen database")?
                .into();

            // TODO: real ol client
            let ol_client = Arc::new(DummyOLClient {
                genesis_epoch: config.params().genesis_ol_epoch_commitment(),
            });

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

            let best_ee_blockhash = {
                #[cfg(feature = "sequencer")]
                {
                    if ext.sequencer {
                        exec_chain_state.tip_blockhash()
                    } else {
                        ol_tracker_state.best_ee_state().last_exec_blkid()
                    }
                }
                #[cfg(not(feature = "sequencer"))]
                {
                    ol_tracker_state.best_ee_state().last_exec_blkid()
                }
            };

            // --- INITIALIZE SERVICES ---

            // Create gossip channel before building the node so we can register it early
            let (gossip_tx, gossip_rx) = mpsc::unbounded_channel();

            // Create preconf channel for p2p head block gossip -> engine control integration
            // This channel sends block hashes received from peers to the engine control task
            let (preconf_tx, preconf_rx) = watch::channel(best_ee_blockhash);

            let (ol_tracker, ol_tracker_task) = OLTrackerBuilder::new(
                ol_tracker_state,
                config.params().clone(),
                storage.clone(),
                ol_client.clone(),
            )
            .build();

            let node_builder = builder
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

            let handle = node_builder.launch().await?;

            let node = handle.node;

            // Sync chainstate to engine for sequencer nodes before starting other tasks
            #[cfg(feature = "sequencer")]
            if ext.sequencer {
                let engine = AlpenRethExecEngine::new(node.beacon_engine_handle.clone());
                let storage_clone = storage.clone();
                let provider_clone = node.provider.clone();

                // Block on the async sync operation
                let sync_result =
                    sync_chainstate_to_engine(storage_clone.as_ref(), &provider_clone, &engine)
                        .await;

                if let Err(e) = sync_result {
                    error!(target: "alpen-client", error = ?e, "failed to sync chainstate to engine on startup");
                    return Err(eyre::eyre!("chainstate sync failed: {e}"));
                }

                info!(target: "alpen-client", "chainstate sync completed successfully");
            }

            let engine_control_task = create_engine_control_task(
                preconf_rx,
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
                // sequencer specific tasks
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

                // TODO: batch assembly
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
    #[arg(long, required = true)]
    pub ol_client_url: String,

    #[arg(long, required = false)]
    pub db_retry_count: Option<u16>,

    /// Run the node as a sequencer. Requires the `sequencer` feature and a
    /// `SEQUENCER_PRIVATE_KEY` environment variable.
    #[arg(long, default_value_t = false)]
    pub sequencer: bool,

    /// Sequencer's public key (hex-encoded, 32 bytes) for signature validation.
    #[arg(long, required = true, value_parser = parse_buf32)]
    pub sequencer_pubkey: Buf32,
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

/// Handle genesis related tasks.
/// Mainly deals with ensuring database has minimal expected state.
async fn ensure_genesis<TStorage: Storage + ExecBlockStorage>(
    config: &AlpenEeConfig,
    storage: &TStorage,
) -> eyre::Result<()> {
    ensure_genesis_ee_account_state(config, storage).await?;
    #[cfg(feature = "sequencer")]
    ensure_finalized_exec_chain_genesis(config, storage).await?;
    // TODO: ensure_batch_genesis after BatchStorage is implemented
    Ok(())
}
